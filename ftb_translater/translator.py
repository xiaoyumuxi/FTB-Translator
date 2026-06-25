from __future__ import annotations

from collections import OrderedDict
import json
from collections.abc import Callable, Mapping
from pathlib import Path

from ftb_translater.backup import create_backup
from ftb_translater.cache import TranslationCache
from ftb_translater.chapters import chapter_files, extract_chapter_segments, replace_chapter_segments
from ftb_translater.deepseek_client import DEFAULT_MODEL, DEFAULT_STYLE, DeepSeekTranslator
from ftb_translater.format_guard import preserved_token_warnings
from ftb_translater.paths import detect_source_mode, source_lang_path, target_lang_path
from ftb_translater.report import TranslationReport
from ftb_translater.snbt import load_lang_snbt, write_lang_snbt


ProgressCallback = Callable[[str, int, int], None]
LogCallback = Callable[[str], None]
AUTO_BATCH_MAX_ENTRIES = 25
AUTO_BATCH_MAX_CHARS = 6000


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
    translator: DeepSeekTranslator | None = None,
) -> TranslationReport:
    if batch_size is not None and batch_size <= 0:
        raise ValueError("Batch size must be greater than zero.")

    source_path = source_lang_path(quests_dir)
    target_path = target_lang_path(quests_dir)
    source_values = load_lang_snbt(source_path)
    cache = TranslationCache(quests_dir / ".ftb-translater" / "cache.json")
    cache.load()

    translated_values: OrderedDict[str, str] = OrderedDict()
    pending: OrderedDict[str, str] = OrderedDict()
    cache_hits = 0
    warnings: dict[str, list[str]] = {}

    for key, value in source_values.items():
        cached = cache.get(value, model, "zh_cn", style)
        if cached is not None:
            translated_values[key] = cached
            cache_hits += 1
        else:
            pending[key] = value

    client = translator or DeepSeekTranslator(api_key=api_key, model=model, logger=logger)
    total_pending = len(pending)
    completed_pending = 0
    failed_entries: list[str] = []
    batches = build_translation_batches(pending, batch_size=batch_size)

    for batch_index, batch in enumerate(batches, start=1):
        if progress:
            progress("translating", completed_pending, total_pending)
        if logger:
            logger(f"DeepSeek batch {batch_index}/{len(batches)}: {len(batch)} lang entries.")
        try:
            result = client.translate_batch(batch, style=style)
        except Exception as exc:  # noqa: BLE001 - report and preserve source text for failed entries.
            if logger:
                logger(f"Batch {batch_index} failed, preserving source text for this batch: {exc}")
            result = {}
            for key in batch:
                failed_entries.append(f"{key}: {exc}")
                result[key] = batch[key]

        for key, source_text in batch.items():
            translated_text = result.get(key, source_text)
            translated_values[key] = translated_text
            if translated_text != source_text:
                cache.set(source_text, model, "zh_cn", style, translated_text)
            token_warnings = preserved_token_warnings(source_text, translated_text)
            if token_warnings:
                warnings[key] = token_warnings
        completed_pending += len(batch)

    ordered_output: OrderedDict[str, str] = OrderedDict()
    for key in source_values:
        ordered_output[key] = translated_values[key]

    if logger:
        logger("Creating backup for lang directory before overwrite.")
    backup_dir = create_backup(quests_dir, directories=("lang",))
    if logger:
        logger(f"Overwriting target lang file: {target_path}")
    write_lang_snbt(target_path, ordered_output)
    parsed_target = load_lang_snbt(target_path)
    if list(parsed_target.keys()) != list(source_values.keys()):
        raise ValueError("Written zh_cn.snbt does not contain the same keys as en_us.snbt.")

    cache.save()
    report = TranslationReport(
        source_file=str(source_path),
        target_file=str(target_path),
        backup_dir=str(backup_dir),
        total_entries=len(source_values),
        translated_entries=len(source_values) - len(failed_entries),
        cache_hits=cache_hits,
        failed_entries=failed_entries,
        warnings=warnings,
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
    translator: DeepSeekTranslator | None = None,
) -> TranslationReport:
    if batch_size is not None and batch_size <= 0:
        raise ValueError("Batch size must be greater than zero.")

    files = chapter_files(quests_dir)
    if not files:
        raise FileNotFoundError(f"No chapter SNBT files found under {quests_dir / 'chapters'}")

    cache = TranslationCache(quests_dir / ".ftb-translater" / "cache.json")
    cache.load()
    segments_by_file = {path: extract_chapter_segments(path) for path in files}
    all_segments = [segment for segments in segments_by_file.values() for segment in segments]

    client = translator or DeepSeekTranslator(api_key=api_key, model=model, logger=logger)
    pending: OrderedDict[str, str] = OrderedDict()
    translations_by_file: dict[Path, dict[int, str]] = {path: {} for path in files}
    cache_hits = 0
    warnings: dict[str, list[str]] = {}

    for segment in all_segments:
        cached = cache.get(segment.source_text, model, "zh_cn", style)
        if cached is not None:
            translations_by_file[segment.path][segment.index] = cached
            cache_hits += 1
        else:
            pending[segment.cache_id] = segment.source_text

    segment_by_id = {segment.cache_id: segment for segment in all_segments}
    batches = build_translation_batches(pending, batch_size=batch_size)
    completed_pending = 0
    failed_entries: list[str] = []

    for batch_index, batch in enumerate(batches, start=1):
        if progress:
            progress("translating", completed_pending, len(pending))
        if logger:
            logger(f"DeepSeek batch {batch_index}/{len(batches)}: {len(batch)} chapter text entries.")
        try:
            result = client.translate_batch(batch, style=style)
        except Exception as exc:  # noqa: BLE001 - report and preserve source text for failed entries.
            if logger:
                logger(f"Batch {batch_index} failed, preserving source text for this batch: {exc}")
            result = {}
            for key, value in batch.items():
                failed_entries.append(f"{key}: {exc}")
                result[key] = value

        for cache_id, source_text in batch.items():
            segment = segment_by_id[cache_id]
            translated_text = result.get(cache_id, source_text)
            translations_by_file[segment.path][segment.index] = translated_text
            if translated_text != source_text:
                cache.set(source_text, model, "zh_cn", style, translated_text)
            token_warnings = preserved_token_warnings(source_text, translated_text)
            if token_warnings:
                warnings[cache_id] = token_warnings
        completed_pending += len(batch)

    if logger:
        logger("Creating backup for chapters directory before overwrite.")
    backup_dir = create_backup(quests_dir, directories=("chapters",))
    replaced_count = 0
    for path, replacements in translations_by_file.items():
        if logger and replacements:
            logger(f"Overwriting chapter file: {path} ({len(replacements)} text segments).")
        replaced_count += replace_chapter_segments(path, replacements)

    cache.save()
    report = TranslationReport(
        source_file=str(quests_dir / "chapters"),
        target_file=str(quests_dir / "chapters"),
        backup_dir=str(backup_dir),
        total_entries=len(all_segments),
        translated_entries=replaced_count - len(failed_entries),
        cache_hits=cache_hits,
        failed_entries=failed_entries,
        warnings=warnings,
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
    translator: DeepSeekTranslator | None = None,
) -> TranslationReport:
    mode = detect_source_mode(quests_dir)
    if mode == "lang":
        return translate_quests_lang(quests_dir, api_key, batch_size, model, style, progress, logger, translator)
    return translate_quests_chapters(quests_dir, api_key, batch_size, model, style, progress, logger, translator)
