from __future__ import annotations

import os
import tempfile
import unittest
from collections import OrderedDict
from pathlib import Path

from ftb_translater.config import load_api_key
from ftb_translater.paths import resolve_quests_dir, source_lang_path, target_lang_path
from ftb_translater.snbt import LangValue, load_lang_snbt, write_lang_snbt
from ftb_translater.translator import translate_quests_auto
from tests.test_live_curseforge import (
    CURSEFORGE_URL_ENV,
    DEFAULT_CURSEFORGE_URL,
    _download,
    _max_download_bytes,
    _safe_extract_zip,
)


LIVE_DEEPSEEK_ENV = "FTB_TRANSLATER_LIVE_DEEPSEEK"
LIVE_DEEPSEEK_ENTRIES_ENV = "FTB_TRANSLATER_LIVE_DEEPSEEK_ENTRIES"
LIVE_OUTPUT_DIR_ENV = "FTB_TRANSLATER_LIVE_OUTPUT_DIR"
DEFAULT_LIVE_DEEPSEEK_ENTRIES = 12


@unittest.skipUnless(
    os.getenv(LIVE_DEEPSEEK_ENV) == "1",
    f"set {LIVE_DEEPSEEK_ENV}=1 to run the paid live DeepSeek end-to-end test",
)
class LiveDeepSeekEndToEndTests(unittest.TestCase):
    def test_download_extract_translate_and_write_with_real_deepseek(self) -> None:
        api_key = load_api_key(Path.cwd())
        if not api_key:
            self.skipTest("DEEPSEEK_API_KEY is not configured")

        url = os.getenv(CURSEFORGE_URL_ENV, DEFAULT_CURSEFORGE_URL)
        sample_size = _sample_size()

        root, cleanup = _working_root()
        try:
            archive = root / "curseforge-pack.zip"
            extract_dir = root / "extracted"

            _download(url, archive, max_bytes=_max_download_bytes())
            _safe_extract_zip(archive, extract_dir)

            quests_dir = resolve_quests_dir(extract_dir)
            source_path = source_lang_path(quests_dir)
            source_values = load_lang_snbt(source_path)
            sampled_values = _sample_lang_values(source_values, sample_size)
            write_lang_snbt(source_path, sampled_values)

            report = translate_quests_auto(
                quests_dir,
                api_key=api_key,
                batch_size=min(6, sample_size),
            )

            translated_values = load_lang_snbt(target_lang_path(quests_dir))
            self.assertEqual(report.total_entries, len(sampled_values))
            self.assertEqual(report.failed_entries, [])
            self.assertEqual(list(translated_values.keys()), list(sampled_values.keys()))
            self.assertTrue(Path(report.backup_dir).exists())
            self.assertTrue((quests_dir / ".ftb-translater" / "cache.json").exists())
            self.assertTrue((quests_dir / ".ftb-translater" / "report-latest.json").exists())
            self.assertTrue(_has_changed_text(sampled_values, translated_values))
            _write_summary(root, quests_dir, report)
        finally:
            if cleanup is not None:
                cleanup.cleanup()


def _working_root() -> tuple[Path, tempfile.TemporaryDirectory[str] | None]:
    output_dir = os.getenv(LIVE_OUTPUT_DIR_ENV)
    if output_dir:
        root = Path(output_dir)
        root.mkdir(parents=True, exist_ok=True)
        return root, None
    cleanup = tempfile.TemporaryDirectory()
    return Path(cleanup.name), cleanup


def _write_summary(root: Path, quests_dir: Path, report) -> None:
    summary = "\n".join(
        [
            f"run_dir={root.resolve()}",
            f"quests_dir={quests_dir.resolve()}",
            f"target={target_lang_path(quests_dir).resolve()}",
            f"report={quests_dir / '.ftb-translater' / 'report-latest.json'}",
            f"cache={quests_dir / '.ftb-translater' / 'cache.json'}",
            f"backup={report.backup_dir}",
            f"total_entries={report.total_entries}",
            f"translated_entries={report.translated_entries}",
            f"cache_hits={report.cache_hits}",
            f"failed_entries={len(report.failed_entries)}",
            f"warnings={len(report.warnings)}",
        ]
    )
    (root / "summary.txt").write_text(summary + "\n", encoding="utf-8")


def _sample_size() -> int:
    raw_value = os.getenv(LIVE_DEEPSEEK_ENTRIES_ENV)
    if raw_value is None:
        return DEFAULT_LIVE_DEEPSEEK_ENTRIES
    try:
        value = int(raw_value)
    except ValueError as exc:
        raise ValueError(f"{LIVE_DEEPSEEK_ENTRIES_ENV} must be an integer") from exc
    if value <= 0:
        raise ValueError(f"{LIVE_DEEPSEEK_ENTRIES_ENV} must be greater than zero")
    return value


def _sample_lang_values(values: OrderedDict[str, LangValue], sample_size: int) -> OrderedDict[str, LangValue]:
    sampled: OrderedDict[str, LangValue] = OrderedDict()
    list_item: tuple[str, LangValue] | None = None
    for key, value in values.items():
        if isinstance(value, list) and list_item is None:
            list_item = (key, value)
        if len(sampled) < sample_size:
            sampled[key] = value
    if list_item is not None and list_item[0] not in sampled:
        if len(sampled) >= sample_size:
            sampled.popitem()
        sampled[list_item[0]] = list_item[1]
    return sampled


def _has_changed_text(source: OrderedDict[str, LangValue], translated: OrderedDict[str, LangValue]) -> bool:
    for key, source_value in source.items():
        if _value_to_text(source_value) != _value_to_text(translated[key]):
            return True
    return False


def _value_to_text(value: LangValue) -> str:
    if isinstance(value, list):
        return "\n".join(value)
    return value


if __name__ == "__main__":
    unittest.main()
