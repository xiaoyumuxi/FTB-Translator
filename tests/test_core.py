from __future__ import annotations

import json
import os
import threading
import time
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from ftb_translater.cache import TranslationCache
from ftb_translater.chapters import count_chapter_segments, extract_chapter_segments
from ftb_translater.config import (
    BASE_URL_KEY,
    BATCH_SIZE_KEY,
    CONCURRENCY_KEY,
    ENV_KEY,
    MODEL_KEY,
    STYLE_KEY,
    env_path,
    load_config_values,
    load_api_key,
    save_config_values,
    save_api_key,
)
from ftb_translater.format_guard import preserved_token_warnings
from ftb_translater.paths import detect_source_mode, resolve_quests_dir
from ftb_translater.snbt import dump_lang_snbt, parse_lang_snbt, write_lang_snbt
from ftb_translater.translator import (
    _resolve_max_workers,
    build_translation_batches,
    estimate_batches,
    translate_quests_auto,
    translate_quests_lang,
)


class FakeTranslator:
    model = "deepseek-v4-flash"

    def translate_batch(self, entries, style):
        return {key: f"汉化:{value}" for key, value in entries.items()}


class UnsafeTranslator:
    model = "deepseek-v4-flash"

    def translate_batch(self, entries, style):
        return {key: "破坏格式的译文" for key in entries}


class TrackingTranslator:
    model = "deepseek-v4-flash"

    def __init__(self):
        self.active = 0
        self.max_active = 0
        self.lock = threading.Lock()

    def translate_batch(self, entries, style):
        with self.lock:
            self.active += 1
            self.max_active = max(self.max_active, self.active)
        try:
            time.sleep(0.05)
            return {key: f"汉化:{value}" for key, value in entries.items()}
        finally:
            with self.lock:
                self.active -= 1


class CoreTests(unittest.TestCase):
    def test_parse_and_dump_lang_snbt_round_trip(self) -> None:
        source = '{\n  "ftbquests.chapter.one": "Hello \\"world\\"\\nLine",\n  bare_key: "Value"\n}\n'
        parsed = parse_lang_snbt(source)

        self.assertEqual(list(parsed.keys()), ["ftbquests.chapter.one", "bare_key"])
        self.assertEqual(parsed["ftbquests.chapter.one"], 'Hello "world"\nLine')
        self.assertEqual(parse_lang_snbt(dump_lang_snbt(parsed)), parsed)

    def test_parse_lang_snbt_accepts_newline_separated_entries(self) -> None:
        source = '{\n  "first": "One"\n  second: "Two"\n}\n'

        self.assertEqual(parse_lang_snbt(source), {"first": "One", "second": "Two"})

    def test_parse_and_dump_lang_snbt_accepts_string_lists(self) -> None:
        source = '{\n  quest_desc: [\n    "First line"\n    ""\n    "Third line"\n  ]\n}\n'
        parsed = parse_lang_snbt(source)

        self.assertEqual(parsed["quest_desc"], ["First line", "", "Third line"])
        self.assertEqual(parse_lang_snbt(dump_lang_snbt(parsed)), parsed)

    def test_resolve_quests_dir_accepts_root_or_quests_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            quests = root / "config" / "ftbquests" / "quests"
            lang = quests / "lang"
            lang.mkdir(parents=True)
            write_lang_snbt(lang / "en_us.snbt", {"a": "A"})

            self.assertEqual(resolve_quests_dir(root), quests.resolve())
            self.assertEqual(resolve_quests_dir(root / "config"), quests.resolve())
            self.assertEqual(resolve_quests_dir(root / "config" / "ftbquests"), quests.resolve())
            self.assertEqual(resolve_quests_dir(quests), quests.resolve())
            self.assertEqual(resolve_quests_dir(lang), quests.resolve())
            self.assertEqual(detect_source_mode(quests), "lang")

    def test_resolve_quests_dir_accepts_chapters_mode(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            chapters = root / "config" / "ftbquests" / "quests" / "chapters"
            chapters.mkdir(parents=True)
            (chapters / "basics.snbt").write_text('title: "Getting Started"\n', encoding="utf-8")

            quests = chapters.parent
            self.assertEqual(resolve_quests_dir(root), quests.resolve())
            self.assertEqual(resolve_quests_dir(chapters), quests.resolve())
            self.assertEqual(detect_source_mode(quests), "chapters")

    def test_resolve_quests_dir_searches_nested_modpack(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            quests = root / "instances" / "Pack A" / "config" / "ftbquests" / "quests"
            chapters = quests / "chapters"
            chapters.mkdir(parents=True)
            (chapters / "basics.snbt").write_text('title: "Getting Started"\n', encoding="utf-8")

            self.assertEqual(resolve_quests_dir(root), quests.resolve())

    def test_env_save_and_load(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            save_api_key("sk-test", root)
            self.assertEqual((root / ".env").read_text(encoding="utf-8").strip(), f"{ENV_KEY}=sk-test")
            self.assertEqual(load_api_key(root), "sk-test")

    def test_default_env_path_uses_config_dir_override(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.dict("os.environ", {"FTB_TRANSLATER_CONFIG_DIR": str(root)}):
                self.assertEqual(env_path(), root / ".env")

    def test_default_load_does_not_read_cwd_env_or_api_key_environment(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            app_config = root / "app-config"
            cwd = root / "cwd"
            app_config.mkdir()
            cwd.mkdir()
            (cwd / ".env").write_text(f"{ENV_KEY}=sk-from-cwd\n", encoding="utf-8")

            previous_cwd = Path.cwd()
            try:
                os.chdir(cwd)
                with patch.dict(
                    "os.environ",
                    {
                        "FTB_TRANSLATER_CONFIG_DIR": str(app_config),
                        ENV_KEY: "sk-from-env",
                    },
                ):
                    self.assertEqual(load_api_key(), "")
                    self.assertEqual(load_config_values()[ENV_KEY], "")
            finally:
                os.chdir(previous_cwd)

    def test_save_and_load_app_config_values_preserves_unrelated_env(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".env").write_text("UNRELATED=keep\n", encoding="utf-8")
            save_config_values(
                {
                    ENV_KEY: "sk-test",
                    BASE_URL_KEY: "https://api.example.test",
                    MODEL_KEY: "deepseek-test",
                    STYLE_KEY: "style",
                    BATCH_SIZE_KEY: "12",
                    CONCURRENCY_KEY: "3",
                },
                root,
            )

            text = (root / ".env").read_text(encoding="utf-8")
            self.assertIn("UNRELATED=keep", text)
            values = load_config_values(root)
            self.assertEqual(values[ENV_KEY], "sk-test")
            self.assertEqual(values[BASE_URL_KEY], "https://api.example.test")
            self.assertEqual(values[MODEL_KEY], "deepseek-test")
            self.assertEqual(values[STYLE_KEY], "style")
            self.assertEqual(values[BATCH_SIZE_KEY], "12")
            self.assertEqual(values[CONCURRENCY_KEY], "3")

    def test_cache_hit(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cache = TranslationCache(Path(tmp) / "cache.json")
            cache.load()
            cache.set("Hello", "deepseek-v4-flash", "zh_cn", "style", "你好")
            cache.save()

            reloaded = TranslationCache(Path(tmp) / "cache.json")
            reloaded.load()
            self.assertEqual(reloaded.get("Hello", "deepseek-v4-flash", "zh_cn", "style"), "你好")

    def test_format_guard_warns_for_missing_placeholder(self) -> None:
        self.assertTrue(preserved_token_warnings("Get %s from <item:minecraft:stone>", "获取石头"))

    def test_format_guard_warns_for_lost_newline(self) -> None:
        self.assertTrue(preserved_token_warnings("Line one\nLine two", "第一行 第二行"))

    def test_translate_quests_lang_integration(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            quests = root / "config" / "ftbquests" / "quests"
            lang = quests / "lang"
            lang.mkdir(parents=True)
            write_lang_snbt(lang / "en_us.snbt", {"title": "Welcome", "desc": "Craft a table"})
            write_lang_snbt(lang / "zh_cn.snbt", {"old": "旧"})

            report = translate_quests_lang(
                quests,
                api_key="unused",
                batch_size=1,
                translator=FakeTranslator(),
            )

            output = parse_lang_snbt((lang / "zh_cn.snbt").read_text(encoding="utf-8"))
            self.assertEqual(output, {"title": "汉化:Welcome", "desc": "汉化:Craft a table"})
            self.assertEqual(report.total_entries, 2)
            self.assertEqual(report.cache_hits, 0)
            self.assertTrue(Path(report.backup_dir, "lang", "zh_cn.snbt").exists())
            self.assertTrue(
                json.loads((quests / ".ftb-translater" / "report-latest.json").read_text(encoding="utf-8"))
            )

    def test_translate_quests_lang_runs_batches_concurrently(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            quests = root / "config" / "ftbquests" / "quests"
            lang = quests / "lang"
            lang.mkdir(parents=True)
            write_lang_snbt(
                lang / "en_us.snbt",
                {"a": "One", "b": "Two", "c": "Three", "d": "Four"},
            )
            translator = TrackingTranslator()

            translate_quests_lang(
                quests,
                api_key="unused",
                batch_size=1,
                translator=translator,
                max_workers=3,
            )

            self.assertGreater(translator.max_active, 1)

    def test_translate_quests_lang_preserves_source_when_format_tokens_are_lost(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            quests = root / "config" / "ftbquests" / "quests"
            lang = quests / "lang"
            lang.mkdir(parents=True)
            source = "Use &e%s&r from <item:minecraft:stone>\\nNext"
            write_lang_snbt(lang / "en_us.snbt", {"desc": source})

            report = translate_quests_lang(
                quests,
                api_key="unused",
                batch_size=1,
                translator=UnsafeTranslator(),
            )

            output = parse_lang_snbt((lang / "zh_cn.snbt").read_text(encoding="utf-8"))
            self.assertEqual(output["desc"], source)
            self.assertTrue(report.warnings["desc"])

    def test_chapter_segments_extract_and_translate_auto(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            quests = root / "config" / "ftbquests" / "quests"
            chapters = quests / "chapters"
            chapters.mkdir(parents=True)
            chapter = chapters / "basics.snbt"
            chapter.write_text(
                """
{
  id: "not translated",
  title: "Getting Started",
  quests: [
    {
      id: "abc123",
      "subtitle": "First Steps",
      description: [
        "Craft a table",
        "",
        "Use <item:minecraft:stone> and %s"
      ],
      icon: "minecraft:stone"
    }
  ]
}
""".strip(),
                encoding="utf-8",
            )

            segments = extract_chapter_segments(chapter)
            self.assertEqual([segment.source_text for segment in segments], [
                "Getting Started",
                "First Steps",
                "Craft a table",
                "Use <item:minecraft:stone> and %s",
            ])
            self.assertEqual(count_chapter_segments(quests), (1, 4))

            report = translate_quests_auto(
                quests,
                api_key="unused",
                batch_size=2,
                translator=FakeTranslator(),
            )
            output = chapter.read_text(encoding="utf-8")
            self.assertIn('"汉化:Getting Started"', output)
            self.assertIn('"汉化:First Steps"', output)
            self.assertIn('"汉化:Craft a table"', output)
            self.assertIn('icon: "minecraft:stone"', output)
            self.assertTrue(Path(report.backup_dir, "chapters", "basics.snbt").exists())
            self.assertTrue(json.loads((quests / ".ftb-translater" / "report-latest.json").read_text()))

    def test_estimate_batches(self) -> None:
        self.assertEqual(estimate_batches(0, 20), 0)
        self.assertEqual(estimate_batches(21, 20), 2)

    def test_auto_translation_batches_split_by_size(self) -> None:
        batches = build_translation_batches(
            {
                "a": "short",
                "b": "x" * 50,
                "c": "y" * 50,
            },
            max_chars=70,
        )
        self.assertEqual(len(batches), 3)

    def test_auto_concurrency_scales_with_task_size(self) -> None:
        self.assertEqual(_resolve_max_workers(None, batch_count=1, entry_count=10), 1)
        self.assertEqual(_resolve_max_workers(None, batch_count=4, entry_count=20), 2)
        self.assertEqual(_resolve_max_workers(None, batch_count=8, entry_count=100), 3)
        self.assertEqual(_resolve_max_workers(None, batch_count=20, entry_count=500), 4)
        self.assertEqual(_resolve_max_workers(None, batch_count=80, entry_count=2500), 6)

    def test_explicit_concurrency_overrides_auto(self) -> None:
        self.assertEqual(_resolve_max_workers(3, batch_count=80, entry_count=2500), 3)


if __name__ == "__main__":
    unittest.main()
