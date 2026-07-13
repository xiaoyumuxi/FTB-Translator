from __future__ import annotations

import json
import os
import threading
import time
import tempfile
import unittest
from collections.abc import Mapping
from pathlib import Path
from unittest.mock import patch

from ftb_translater.cache import TranslationCache
from ftb_translater.chapters import count_chapter_segments, extract_chapter_segments, replace_chapter_segments
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
)
from ftb_translater.deepseek_client import DeepSeekTranslator
from ftb_translater.format_guard import preserved_token_warnings, protect_text, repair_translation_format, restore_text
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

    def translate_batch(self, entries: Mapping[str, str], style: str) -> dict[str, str]:
        return {key: f"汉化:{value}" for key, value in entries.items()}


class UnsafeTranslator:
    model = "deepseek-v4-flash"

    def translate_batch(self, entries: Mapping[str, str], style: str) -> dict[str, str]:
        return {key: "破坏格式的译文" for key in entries}


class FailingTranslator:
    model = "deepseek-v4-flash"

    def translate_batch(self, entries: Mapping[str, str], style: str) -> dict[str, str]:
        raise RuntimeError("network down")


class TrackingTranslator:
    model = "deepseek-v4-flash"

    def __init__(self):
        self.active = 0
        self.max_active = 0
        self.lock = threading.Lock()

    def translate_batch(self, entries: Mapping[str, str], style: str) -> dict[str, str]:
        with self.lock:
            self.active += 1
            self.max_active = max(self.max_active, self.active)
        try:
            time.sleep(0.05)
            return {key: f"汉化:{value}" for key, value in entries.items()}
        finally:
            with self.lock:
                self.active -= 1


class RecordingProtectedTranslator:
    model = "deepseek-v4-flash"

    def __init__(self):
        self.seen_values: list[str] = []

    def translate_batch(self, entries: Mapping[str, str], style: str) -> dict[str, str]:
        self.seen_values.extend(entries.values())
        return {
            key: value.replace("Defeat", "击败")
            .replace("Ignis", "伊格尼斯")
            .replace("Burning Arena", "燃烧竞技场")
            .replace("in the Lava Dimension", "在熔岩维度")
            for key, value in entries.items()
        }


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

    def test_legacy_env_load(self) -> None:
        # load_api_key 现在仅用于读取旧 .env 进行迁移
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".env").write_text(f"{ENV_KEY}=sk-legacy\n", encoding="utf-8")
            self.assertEqual(load_api_key(root), "sk-legacy")

    def test_migrate_api_key_from_env(self) -> None:
        from ftb_translater import credential_store
        from ftb_translater.config import migrate_api_key_from_env
        from unittest.mock import patch

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".env").write_text(
                f"{ENV_KEY}=sk-old-key\n"
                f"{BASE_URL_KEY}=https://keep.me\n"
                "UNRELATED=stay\n",
                encoding="utf-8",
            )

            saved: dict[str, str] = {}

            def fake_save(key: str) -> None:
                saved["key"] = key

            def fake_load() -> str:
                return saved.get("key", "")

            with patch.object(credential_store, "save_api_key", fake_save), \
                 patch.object(credential_store, "load_api_key", fake_load):
                changed = migrate_api_key_from_env(root)
                self.assertTrue(changed)
                self.assertEqual(saved.get("key"), "sk-old-key")

            remaining = (root / ".env").read_text(encoding="utf-8")
            self.assertNotIn(f"{ENV_KEY}=", remaining)
            self.assertIn(f"{BASE_URL_KEY}=https://keep.me", remaining)
            self.assertIn("UNRELATED=stay", remaining)

    def test_migrate_api_key_keeps_different_env_key_when_already_in_store(self) -> None:
        from ftb_translater import credential_store
        from ftb_translater.config import migrate_api_key_from_env
        from unittest.mock import patch

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".env").write_text(f"{ENV_KEY}=sk-stale\n", encoding="utf-8")

            with patch.object(credential_store, "load_api_key", return_value="sk-existing"), \
                 patch.object(credential_store, "save_api_key") as save_mock:
                changed = migrate_api_key_from_env(root)
                self.assertFalse(changed)
                save_mock.assert_not_called()

            # 如果 .env 是不同的 key,不能静默删除,留给用户人工确认。
            self.assertIn(f"{ENV_KEY}=sk-stale", (root / ".env").read_text(encoding="utf-8"))

    def test_migrate_api_key_strips_duplicate_env_key_when_already_in_store(self) -> None:
        from ftb_translater import credential_store
        from ftb_translater.config import migrate_api_key_from_env
        from unittest.mock import patch

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".env").write_text(f"{ENV_KEY}=sk-same\n", encoding="utf-8")

            with patch.object(credential_store, "load_api_key", return_value="sk-same"), \
                 patch.object(credential_store, "save_api_key") as save_mock:
                changed = migrate_api_key_from_env(root)
                self.assertFalse(changed)
                save_mock.assert_not_called()

            self.assertNotIn(f"{ENV_KEY}=", (root / ".env").read_text(encoding="utf-8"))

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
                    # API key 不再属于 APP_CONFIG_KEYS,所以 load_config_values 不会返回它
                    self.assertNotIn(ENV_KEY, load_config_values())
            finally:
                os.chdir(previous_cwd)

    def test_save_and_load_app_config_values_preserves_unrelated_env(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".env").write_text("UNRELATED=keep\n", encoding="utf-8")
            save_config_values(
                {
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
            # API key 不再写入 .env
            self.assertNotIn(f"{ENV_KEY}=", text)
            values = load_config_values(root)
            self.assertEqual(values[BASE_URL_KEY], "https://api.example.test")
            self.assertEqual(values[MODEL_KEY], "deepseek-test")
            self.assertEqual(values[STYLE_KEY], "style")
            self.assertEqual(values[BATCH_SIZE_KEY], "12")
            self.assertEqual(values[CONCURRENCY_KEY], "3")
            self.assertNotIn(ENV_KEY, values)

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

    def test_format_guard_allows_colour_segments_to_move(self) -> None:
        source = "Defeat &cIgnis&r in the &cBurning Arena&r with &dAshes&r"
        translated = "在&c燃烧竞技场&r中用&d灰烬&r击败&c伊格尼斯&r"

        self.assertEqual(preserved_token_warnings(source, translated), [])

    def test_format_guard_warns_for_invalid_colour_ast_order(self) -> None:
        source = "Defeat &cIgnis&r"
        translated = "击败 &r伊格尼斯&c"

        warnings = preserved_token_warnings(source, translated)

        self.assertTrue(any("Colour/style AST" in warning for warning in warnings))

    def test_format_guard_warns_when_colour_modifier_ast_changes(self) -> None:
        source = "&cRed &lBold&r"
        translated = "&l加粗 &c红色&r"

        warnings = preserved_token_warnings(source, translated)

        self.assertTrue(any("Colour/style AST" in warning for warning in warnings))

    def test_format_guard_repairs_extra_colour_codes(self) -> None:
        source = "&fSpeak With The &dEcho of The Quartermaster"
        translated = "&f与&d军需官的回响&f对话"

        repaired = repair_translation_format(source, translated)

        self.assertEqual(repaired, "&f与&d军需官的回响对话")
        self.assertEqual(preserved_token_warnings(source, repaired), [])

    def test_format_guard_still_requires_non_colour_token_order(self) -> None:
        source = "Use %s on <item:minecraft:stone>"
        translated = "对<item:minecraft:stone>使用%s"

        self.assertTrue(preserved_token_warnings(source, translated))

    def test_format_guard_protects_ftb_macros_and_resource_paths(self) -> None:
        source = (
            "See {@pagebreak}, ftb:textures/quests/mekanism/portal_frame.png, "
            "and {image:ftb:textures/quests/mekanism/portal_frame.png width:100 height:100 align:center}"
        )

        protected, protections = protect_text(source)
        self.assertNotIn("pagebreak", protected)
        self.assertNotIn("portal_frame.png", protected)

        translated = protected.replace("See", "查看")
        restored = restore_text(translated, protections)
        self.assertIn("{@pagebreak}", restored)
        self.assertIn("ftb:textures/quests/mekanism/portal_frame.png", restored)
        self.assertIn("{image:ftb:textures/quests/mekanism/portal_frame.png width:100 height:100 align:center}", restored)
        self.assertEqual(preserved_token_warnings(source, restored), [])

    def test_format_guard_does_not_protect_common_slash_phrases(self) -> None:
        source = "Transfer items and/or fluids with input/output, Up/Down, I/O, and RF/t labels."

        protected, protections = protect_text(source)

        self.assertEqual(protected, source)
        self.assertEqual(protections, [])
        self.assertEqual(
            preserved_token_warnings(source, "传输物品或流体，并显示输入输出、上下、IO 和 RF/t 标签。"),
            [],
        )

    def test_deepseek_prompt_emphasizes_placeholder_wrappers(self) -> None:
        prompt = DeepSeekTranslator._build_prompt({"desc": "Defeat ⟨P_0⟩Ignis⟨P_1⟩"}, "style")

        self.assertIn("Every placeholder from the input value must appear", prompt)
        self.assertIn("每个占位符", prompt)
        self.assertIn("Translate the word but keep both wrappers", prompt)
        self.assertIn("占位符包住的英文也必须翻译", prompt)
        self.assertIn("⟨P_0⟩下界⟨P_1⟩", prompt)
        self.assertIn("lost closing wrapper", prompt)

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
            saved_report = json.loads((quests / ".ftb-translater" / "report-latest.json").read_text(encoding="utf-8"))
            self.assertTrue(saved_report)

            # 新增字段:mapping 和 output_files(用相对整合包根的路径,导出 ZIP 直接可用)
            self.assertIn("lang/zh_cn.snbt", report.mapping)
            self.assertEqual(report.mapping["lang/zh_cn.snbt"]["title"]["en"], "Welcome")
            self.assertEqual(report.mapping["lang/zh_cn.snbt"]["title"]["zh"], "汉化:Welcome")
            self.assertIn("lang/zh_cn.snbt", report.output_files)
            self.assertIn("汉化:Welcome", report.output_files["lang/zh_cn.snbt"])
            self.assertEqual(saved_report["mapping"]["lang/zh_cn.snbt"]["title"]["zh"], "汉化:Welcome")
            self.assertNotIn("output_files", saved_report)

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

    def test_translate_quests_lang_reports_failed_mapping_details(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            quests = root / "config" / "ftbquests" / "quests"
            lang = quests / "lang"
            lang.mkdir(parents=True)
            write_lang_snbt(lang / "en_us.snbt", {"desc": "Use a stone"})

            report = translate_quests_lang(
                quests,
                api_key="unused",
                batch_size=1,
                translator=FailingTranslator(),
            )

            output = parse_lang_snbt((lang / "zh_cn.snbt").read_text(encoding="utf-8"))
            self.assertEqual(output["desc"], "Use a stone")
            self.assertIn("desc", report.warnings)
            self.assertEqual(report.failed_translations["desc"]["source"], "Use a stone")
            self.assertEqual(report.failed_translations["desc"]["failed"], "")
            self.assertIn("network down", report.failed_translations["desc"]["error"])

    def test_translate_quests_lang_sends_protected_text_to_model(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            quests = root / "config" / "ftbquests" / "quests"
            lang = quests / "lang"
            lang.mkdir(parents=True)
            source = "Defeat &cIgnis&r in the &cBurning Arena&r in the Lava Dimension."
            write_lang_snbt(lang / "en_us.snbt", {"desc": source})
            translator = RecordingProtectedTranslator()

            report = translate_quests_lang(
                quests,
                api_key="unused",
                batch_size=1,
                translator=translator,
            )

            output = parse_lang_snbt((lang / "zh_cn.snbt").read_text(encoding="utf-8"))
            self.assertEqual(output["desc"], "击败 &c伊格尼斯&r in the &c燃烧竞技场&r 在熔岩维度.")
            self.assertEqual(report.warnings, {})
            self.assertTrue(translator.seen_values)
            self.assertNotIn("&c", translator.seen_values[0])
            self.assertNotIn("&r", translator.seen_values[0])
            self.assertIn("Ignis", translator.seen_values[0])

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

    def test_chapter_segments_parse_nested_values_without_false_positives(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            chapter = Path(tmp) / "nested.snbt"
            chapter.write_text(
                """
{
  quests: [
    {
      description: [
        "Craft a table",
        { id: "internal English", text: "Visible Text", name: "Display Name" },
        ["Nested Line"]
      ],
      dependencies: ["Not A Task Line"],
      icon: "minecraft:stone"
    }
  ]
}
""".strip(),
                encoding="utf-8",
            )

            segments = extract_chapter_segments(chapter)

            self.assertEqual(
                [(segment.key, segment.source_text) for segment in segments],
                [
                    ("description", "Craft a table"),
                    ("text", "Visible Text"),
                    ("name", "Display Name"),
                    ("description", "Nested Line"),
                ],
            )

    def test_chapter_segment_replacement_uses_parsed_spans_only(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            chapter = Path(tmp) / "replace.snbt"
            chapter.write_text(
                """
{
  title: 'First Quest',
  description: [
    { id: "internal English", text: "Visible Text" }
  ]
}
""".strip(),
                encoding="utf-8",
            )

            replaced = replace_chapter_segments(chapter, {1: "可见文本"})
            output = chapter.read_text(encoding="utf-8")

            self.assertEqual(replaced, 1)
            self.assertIn("title: 'First Quest'", output)
            self.assertIn('id: "internal English"', output)
            self.assertIn('text: "可见文本"', output)

    def test_chapter_segments_accept_translation_table_keys(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            chapter = Path(tmp) / "translations.snbt"
            chapter.write_text(
                """
{
  "quest.0123456789ABCDEF.title": "Quest Title",
  quest.0123456789ABCDEF.quest_desc: [
    "First description line",
    "",
    "Second description line"
  ],
  "chapter.1111111111111111.chapter_subtitle": ["Chapter Subtitle"],
  "quest.0123456789ABCDEF.dependencies": ["Not a title"]
}
""".strip(),
                encoding="utf-8",
            )

            segments = extract_chapter_segments(chapter)

            self.assertEqual(
                [(segment.key, segment.source_text) for segment in segments],
                [
                    ("quest.0123456789ABCDEF.title", "Quest Title"),
                    ("quest.0123456789ABCDEF.quest_desc", "First description line"),
                    ("quest.0123456789ABCDEF.quest_desc", "Second description line"),
                    ("chapter.1111111111111111.chapter_subtitle", "Chapter Subtitle"),
                ],
            )

    def test_chapter_segments_accept_initial_translation_blocks(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            chapter = Path(tmp) / "extra.snbt"
            chapter.write_text(
                """
{
  extra: {
    locale: "en_us",
    translate: {
      title: "Initial Title",
      quest_desc: ["Initial Description"],
      invalid_key: "Do Not Translate"
    }
  }
}
""".strip(),
                encoding="utf-8",
            )

            segments = extract_chapter_segments(chapter)

            self.assertEqual(
                [(segment.key, segment.source_text) for segment in segments],
                [
                    ("title", "Initial Title"),
                    ("quest_desc", "Initial Description"),
                ],
            )

    def test_chapter_segments_skip_reference_style_values(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            chapter = Path(tmp) / "references.snbt"
            chapter.write_text(
                """
{
  quests: [{
    description: "ftb.shop.notification.guidance",
    text: "This should be translated",
    name: "minecraft:stone",
    subtitle: "echo_guidance_meet"
  }]
}
""".strip(),
                encoding="utf-8",
            )

            segments = extract_chapter_segments(chapter)

            self.assertEqual(
                [(segment.key, segment.source_text) for segment in segments],
                [("text", "This should be translated")],
            )

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
