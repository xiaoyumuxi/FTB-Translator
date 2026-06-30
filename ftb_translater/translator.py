from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor, as_completed
from collections import OrderedDict
import json
from collections.abc import Callable, Mapping
import os
from pathlib import Path
import threading
from dataclasses import dataclass
from typing import Protocol, cast

from ftb_translater.backup import create_backup
from ftb_translater.cache import TranslationCache
from ftb_translater.chapters import chapter_files, extract_chapter_segments, replace_chapter_segments
from ftb_translater.deepseek_client import DEFAULT_BASE_URL, DEFAULT_MODEL, DEFAULT_STYLE, DeepSeekTranslator
from ftb_translater.format_guard import preserved_token_warnings, protect_text, restore_text
from ftb_translater.logger import get_logger
from ftb_translater.paths import detect_source_mode, source_lang_path, target_lang_path
from ftb_translater.report import TranslationReport
from ftb_translater.snbt import LangValue, load_lang_snbt, write_lang_snbt

_log = get_logger(__name__)


ProgressCallback = Callable[[str, int, int], None]
LogCallback = Callable[[str], None]
AUTO_BATCH_MAX_ENTRIES = 25
AUTO_BATCH_MAX_CHARS = 6000
AUTO_MAX_WORKERS = 6
MAX_WORKERS_ENV = "FTB_TRANSLATER_CONCURRENCY"


class TranslatorClient(Protocol):
    def translate_batch(self, entries: Mapping[str, str], style: str) -> dict[str, str]: ...


@dataclass(frozen=True)
class _BatchResult:
    batch_index: int
    batch: OrderedDict[str, str]
    result: dict[str, str]
    error: Exception | None = None


def estimate_batches(total_entries: int, batch_size: int) -> int:
    if batch_size <= 0:
        raise ValueError("Batch size must be greater than zero.")
    return (total_entries + batch_size - 1) // batch_size


def build_translation_batches(
    entries: Mapping[str, str],
    batch_size: int | None = None,
    max_chars: int = AUTO_BATCH_MAX_CHARS,
) -> list[OrderedDict[str, str]]:
    if batch_size is not None and batch_size <= 0:
        raise ValueError("Batch size must be greater than zero.")
    max_entries = batch_size or AUTO_BATCH_MAX_ENTRIES
    batches: list[OrderedDict[str, str]] = []
    current: OrderedDict[str, str] = OrderedDict()
    current_chars = 0

    for key, value in entries.items():
        estimated_chars = len(json.dumps({key: value}, ensure_ascii=False))
        if current and (len(current) >= max_entries or current_chars + estimated_chars > max_chars):
            batches.append(current)
            current = OrderedDict()
            current_chars = 0
        current[key] = value
        current_chars += estimated_chars

    if current:
        batches.append(current)
    return batches


def translate_quests_lang(
    quests_dir: Path,
    api_key: str,
    batch_size: int | None = None,
    model: str = DEFAULT_MODEL,
    style: str = DEFAULT_STYLE,
    progress: ProgressCallback | None = None,
    logger: LogCallback | None = None,
    translator: TranslatorClient | None = None,
    max_workers: int | None = None,
    base_url: str = DEFAULT_BASE_URL,
) -> TranslationReport:
    if batch_size is not None and batch_size <= 0:
        raise ValueError("Batch size must be greater than zero.")

    source_path = source_lang_path(quests_dir)
    target_path = target_lang_path(quests_dir)
    _log.info("translate_quests_lang: quests_dir=%s source=%s", quests_dir, source_path)

    source_values = load_lang_snbt(source_path)
    _log.debug("Loaded source lang: %d entries", len(source_values))

    cache = TranslationCache(quests_dir / ".ftb-translater" / "cache.json")
    cache.load()

    translated_values: OrderedDict[str, LangValue] = OrderedDict()
    pending: OrderedDict[str, str] = OrderedDict()
    pending_sources: OrderedDict[str, str] = OrderedDict()
    protections_by_key: dict[str, list[tuple[str, str]]] = {}
    cache_hits = 0
    warnings: dict[str, list[str]] = {}
    failed_translations: dict[str, dict[str, str]] = {}

    for key, value in source_values.items():
        source_text = _lang_value_to_text(value)
        cached = cache.get(source_text, model, "zh_cn", style)
        if cached is not None:
            translated_values[key] = _text_to_lang_value(cached, value)
            cache_hits += 1
        else:
            protected_text, protections = protect_text(source_text)
            pending[key] = protected_text
            pending_sources[key] = source_text
            protections_by_key[key] = protections

    _log.info("Cache hits: %d, pending translation: %d", cache_hits, len(pending))

    total_pending = len(pending)
    failed_entries: list[str] = []
    batches = build_translation_batches(pending, batch_size=batch_size)
    _log.info("Built %d batches for %d pending entries", len(batches), total_pending)

    batch_results = _translate_batches(
        batches=batches,
        api_key=api_key,
        model=model,
        style=style,
        base_url=base_url,
        logger=logger,
        progress=progress,
        progress_total=total_pending,
        label="lang entries",
        translator=translator,
        max_workers=max_workers,
    )
    for batch_result in sorted(batch_results, key=lambda item: item.batch_index):
        batch = batch_result.batch
        result = batch_result.result
        if batch_result.error is not None:
            for key in batch:
                error_text = f"API batch failed: {batch_result.error}"
                failed_entries.append(f"{key}: {batch_result.error}")
                warnings[key] = [error_text]
                failed_translations[key] = {"source": pending_sources[key], "failed": "", "error": error_text}
        for key, protected_text in batch.items():
            source_text = pending_sources[key]
            translated_text = result.get(key, protected_text)
            model_raw = translated_text
            translated_text, token_warnings = _guard_translation(source_text, translated_text, protections_by_key[key])
            translated_values[key] = _text_to_lang_value(translated_text, source_values[key])
            if translated_text != source_text:
                cache.set(source_text, model, "zh_cn", style, translated_text)
            if token_warnings:
                _log.warning("Format token mismatch for key %r: %s", key, token_warnings)
                warnings[key] = token_warnings
                failed_translations[key] = {"source": source_text, "failed": restore_text(model_raw, protections_by_key[key])}

    ordered_output: OrderedDict[str, LangValue] = OrderedDict()
    for key in source_values:
        ordered_output[key] = translated_values[key]

    msg = "Creating backup for lang directory before overwrite."
    _log.info(msg)
    if logger:
        logger(msg)
    backup_dir = create_backup(quests_dir, directories=("lang",))
    _log.info("Backup created at: %s", backup_dir)

    msg = f"Overwriting target lang file: {target_path}"
    _log.info(msg)
    if logger:
        logger(msg)
    write_lang_snbt(target_path, ordered_output)

    parsed_target = load_lang_snbt(target_path)
    if list(parsed_target.keys()) != list(source_values.keys()):
        _log.error(
            "Key mismatch after write! source keys=%d written keys=%d",
            len(source_values), len(parsed_target),
        )
        raise ValueError("Written zh_cn.snbt does not contain the same keys as en_us.snbt.")

    cache.save()
    _log.info(
        "Lang translation done: total=%d translated=%d cache_hits=%d failed=%d warnings=%d",
        len(source_values), len(source_values) - len(failed_entries),
        cache_hits, len(failed_entries), len(warnings),
    )
    if failed_entries:
        _log.warning("Failed entries:\n%s", "\n".join(failed_entries))

    report = TranslationReport(
        source_file=str(source_path),
        target_file=str(target_path),
        backup_dir=str(backup_dir),
        total_entries=len(source_values),
        translated_entries=len(source_values) - len(failed_entries),
        cache_hits=cache_hits,
        failed_entries=failed_entries,
        warnings=warnings,
        failed_translations=failed_translations,
    )
    report.save(quests_dir)
    if progress:
        progress("done", total_pending, total_pending)
    return report


def translate_quests_chapters(
    quests_dir: Path,
    api_key: str,
    batch_size: int | None = None,
    model: str = DEFAULT_MODEL,
    style: str = DEFAULT_STYLE,
    progress: ProgressCallback | None = None,
    logger: LogCallback | None = None,
    translator: TranslatorClient | None = None,
    max_workers: int | None = None,
    base_url: str = DEFAULT_BASE_URL,
) -> TranslationReport:
    if batch_size is not None and batch_size <= 0:
        raise ValueError("Batch size must be greater than zero.")

    files = chapter_files(quests_dir)
    if not files:
        _log.error("No chapter SNBT files found under %s", quests_dir / "chapters")
        raise FileNotFoundError(f"No chapter SNBT files found under {quests_dir / 'chapters'}")

    _log.info("translate_quests_chapters: quests_dir=%s, files=%d", quests_dir, len(files))

    cache = TranslationCache(quests_dir / ".ftb-translater" / "cache.json")
    cache.load()
    segments_by_file = {path: extract_chapter_segments(path) for path in files}
    all_segments = [segment for segments in segments_by_file.values() for segment in segments]
    _log.debug("Extracted %d total text segments from %d chapter files", len(all_segments), len(files))

    pending: OrderedDict[str, str] = OrderedDict()
    pending_sources: OrderedDict[str, str] = OrderedDict()
    protections_by_key: dict[str, list[tuple[str, str]]] = {}
    translations_by_file: dict[Path, dict[int, str]] = {path: {} for path in files}
    cache_hits = 0
    warnings: dict[str, list[str]] = {}
    failed_translations: dict[str, dict[str, str]] = {}

    for segment in all_segments:
        cached = cache.get(segment.source_text, model, "zh_cn", style)
        if cached is not None:
            translations_by_file[segment.path][segment.index] = cached
            cache_hits += 1
        else:
            protected_text, protections = protect_text(segment.source_text)
            pending[segment.cache_id] = protected_text
            pending_sources[segment.cache_id] = segment.source_text
            protections_by_key[segment.cache_id] = protections

    _log.info("Cache hits: %d, pending translation: %d", cache_hits, len(pending))

    segment_by_id = {segment.cache_id: segment for segment in all_segments}
    batches = build_translation_batches(pending, batch_size=batch_size)
    _log.info("Built %d batches for %d pending segments", len(batches), len(pending))
    failed_entries: list[str] = []

    batch_results = _translate_batches(
        batches=batches,
        api_key=api_key,
        model=model,
        style=style,
        base_url=base_url,
        logger=logger,
        progress=progress,
        progress_total=len(pending),
        label="chapter text entries",
        translator=translator,
        max_workers=max_workers,
    )
    for batch_result in sorted(batch_results, key=lambda item: item.batch_index):
        batch = batch_result.batch
        result = batch_result.result
        if batch_result.error is not None:
            for key in batch:
                error_text = f"API batch failed: {batch_result.error}"
                failed_entries.append(f"{key}: {batch_result.error}")
                warnings[key] = [error_text]
                failed_translations[key] = {"source": pending_sources[key], "failed": "", "error": error_text}
        for cache_id, protected_text in batch.items():
            segment = segment_by_id[cache_id]
            source_text = pending_sources[cache_id]
            translated_text = result.get(cache_id, protected_text)
            model_raw = translated_text
            translated_text, token_warnings = _guard_translation(source_text, translated_text, protections_by_key[cache_id])
            translations_by_file[segment.path][segment.index] = translated_text
            if translated_text != source_text:
                cache.set(source_text, model, "zh_cn", style, translated_text)
            if token_warnings:
                _log.warning("Format token mismatch for segment %r: %s", cache_id, token_warnings)
                warnings[cache_id] = token_warnings
                failed_translations[cache_id] = {"source": source_text, "failed": restore_text(model_raw, protections_by_key[cache_id])}

    msg = "Creating backup for chapters directory before overwrite."
    _log.info(msg)
    if logger:
        logger(msg)
    backup_dir = create_backup(quests_dir, directories=("chapters",))
    _log.info("Backup created at: %s", backup_dir)

    replaced_count = 0
    for path, replacements in translations_by_file.items():
        if replacements:
            msg = f"Overwriting chapter file: {path} ({len(replacements)} text segments)."
            _log.info(msg)
            if logger:
                logger(msg)
        replaced_count += replace_chapter_segments(path, replacements)

    cache.save()
    _log.info(
        "Chapters translation done: total=%d replaced=%d cache_hits=%d failed=%d warnings=%d",
        len(all_segments), replaced_count, cache_hits, len(failed_entries), len(warnings),
    )
    if failed_entries:
        _log.warning("Failed entries:\n%s", "\n".join(failed_entries))

    report = TranslationReport(
        source_file=str(quests_dir / "chapters"),
        target_file=str(quests_dir / "chapters"),
        backup_dir=str(backup_dir),
        total_entries=len(all_segments),
        translated_entries=replaced_count - len(failed_entries),
        cache_hits=cache_hits,
        failed_entries=failed_entries,
        warnings=warnings,
        failed_translations=failed_translations,
    )
    report.save(quests_dir)
    if progress:
        progress("done", len(pending), len(pending))
    return report


def translate_quests_auto(
    quests_dir: Path,
    api_key: str,
    batch_size: int | None = None,
    model: str = DEFAULT_MODEL,
    style: str = DEFAULT_STYLE,
    progress: ProgressCallback | None = None,
    logger: LogCallback | None = None,
    translator: TranslatorClient | None = None,
    max_workers: int | None = None,
    base_url: str = DEFAULT_BASE_URL,
) -> TranslationReport:
    mode = detect_source_mode(quests_dir)
    _log.info("translate_quests_auto: mode=%s quests_dir=%s", mode, quests_dir)
    if mode == "lang":
        return translate_quests_lang(
            quests_dir=quests_dir,
            api_key=api_key,
            batch_size=batch_size,
            model=model,
            style=style,
            base_url=base_url,
            progress=progress,
            logger=logger,
            translator=translator,
            max_workers=max_workers,
        )
    return translate_quests_chapters(
        quests_dir=quests_dir,
        api_key=api_key,
        batch_size=batch_size,
        model=model,
        style=style,
        base_url=base_url,
        progress=progress,
        logger=logger,
        translator=translator,
        max_workers=max_workers,
    )


def _translate_batches(
    batches: list[OrderedDict[str, str]],
    api_key: str,
    model: str,
    style: str,
    base_url: str,
    logger: LogCallback | None,
    progress: ProgressCallback | None,
    progress_total: int,
    label: str,
    translator: TranslatorClient | None,
    max_workers: int | None,
) -> list[_BatchResult]:
    worker_count = _resolve_max_workers(max_workers, len(batches), progress_total)
    if not batches:
        if progress:
            progress("done", 0, progress_total)
        return []

    if logger:
        logger(f"DeepSeek concurrency: {worker_count} worker(s), {len(batches)} batches.")
    _log.info("DeepSeek concurrency: %d worker(s), %d batches", worker_count, len(batches))

    completed_entries = 0
    results: list[_BatchResult] = []

    if worker_count == 1:
        for batch_index, batch in enumerate(batches, start=1):
            if progress:
                progress("translating", completed_entries, progress_total)
            batch_result = _translate_one_batch(
                batch_index=batch_index,
                batch_count=len(batches),
                batch=batch,
                api_key=api_key,
                model=model,
                style=style,
                base_url=base_url,
                logger=logger,
                label=label,
                translator=translator,
            )
            results.append(batch_result)
            completed_entries += len(batch)
        if progress:
            progress("translating", completed_entries, progress_total)
        return results

    thread_state = threading.local()

    def worker(batch_index: int, batch: OrderedDict[str, str]) -> _BatchResult:
        worker_translator = translator
        if worker_translator is None:
            worker_translator = cast(TranslatorClient | None, getattr(thread_state, "translator", None))
            if worker_translator is None:
                worker_translator = DeepSeekTranslator(api_key=api_key, model=model, base_url=base_url, logger=logger)
                thread_state.translator = worker_translator
        return _translate_one_batch(
            batch_index=batch_index,
            batch_count=len(batches),
            batch=batch,
            api_key=api_key,
            model=model,
            style=style,
            base_url=base_url,
            logger=logger,
            label=label,
            translator=worker_translator,
        )

    with ThreadPoolExecutor(max_workers=worker_count) as executor:
        futures = {
            executor.submit(worker, batch_index, batch): batch
            for batch_index, batch in enumerate(batches, start=1)
        }
        for future in as_completed(futures):
            batch_result = future.result()
            results.append(batch_result)
            completed_entries += len(batch_result.batch)
            if progress:
                progress("translating", completed_entries, progress_total)
    return results


def _translate_one_batch(
    batch_index: int,
    batch_count: int,
    batch: OrderedDict[str, str],
    api_key: str,
    model: str,
    style: str,
    base_url: str,
    logger: LogCallback | None,
    label: str,
    translator: TranslatorClient | None,
) -> _BatchResult:
    client = translator or DeepSeekTranslator(api_key=api_key, model=model, base_url=base_url, logger=logger)
    msg = f"DeepSeek batch {batch_index}/{batch_count}: {len(batch)} {label}."
    _log.info(msg)
    if logger:
        logger(msg)
    try:
        return _BatchResult(batch_index=batch_index, batch=batch, result=client.translate_batch(batch, style=style))
    except Exception as exc:  # noqa: BLE001
        msg = f"Batch {batch_index} failed, preserving source text for this batch: {exc}"
        _log.error(msg)
        if logger:
            logger(msg)
        return _BatchResult(batch_index=batch_index, batch=batch, result=dict(batch), error=exc)


def _resolve_max_workers(max_workers: int | None, batch_count: int, entry_count: int = 0) -> int:
    if batch_count <= 0:
        return 1
    value = max_workers
    if value is None:
        raw_value = os.getenv(MAX_WORKERS_ENV)
        if raw_value:
            if raw_value.strip().lower() == "auto":
                return _auto_max_workers(batch_count, entry_count)
            try:
                value = int(raw_value)
            except ValueError:
                _log.warning("%s must be an integer or 'auto', falling back to automatic concurrency", MAX_WORKERS_ENV)
                return _auto_max_workers(batch_count, entry_count)
        else:
            return _auto_max_workers(batch_count, entry_count)
    if value <= 0:
        raise ValueError("max_workers must be greater than zero.")
    return min(value, batch_count)


def _auto_max_workers(batch_count: int, entry_count: int) -> int:
    if batch_count <= 1:
        return 1
    if entry_count <= 25:
        return min(2, batch_count)
    if entry_count <= 150:
        return min(3, batch_count)
    if entry_count <= 800:
        return min(4, batch_count)
    return min(AUTO_MAX_WORKERS, batch_count)


def _guard_translation(
    source_text: str,
    translated_text: str,
    protections: list[tuple[str, str]],
) -> tuple[str, list[str]]:
    # Restore the exact tokens that were removed before sending text to the model.
    restored = restore_text(translated_text, protections)

    token_warnings = preserved_token_warnings(source_text, restored)
    if token_warnings:
        return source_text, [*token_warnings, "Unsafe translation discarded; source text preserved."]
    return restored, []


def _lang_value_to_text(value: LangValue) -> str:
    if isinstance(value, list):
        return "\n".join(value)
    return value


def _text_to_lang_value(text: str, template: LangValue) -> LangValue:
    if isinstance(template, list):
        return text.split("\n")
    return text
