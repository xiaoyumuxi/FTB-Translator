from __future__ import annotations

import json
import tempfile
import unittest
import zipfile
from pathlib import Path

from ftb_translater.history_db import DEFAULT_HISTORY_DB_NAME, FileRecord, HistoryDB


def _sample_files() -> list[FileRecord]:
    return [
        FileRecord(
            filename="zh_cn.snbt",
            source_hash="abc123",
            mapping={
                "quest.1.title": {"en": "First Quest", "zh": "第一个任务"},
                "quest.1.desc": {"en": "Find iron", "zh": "找到铁"},
            },
            output_content='{\n  "quest.1.title": "第一个任务"\n}\n',
        ),
    ]


class HistoryDBTests(unittest.TestCase):
    def test_default_path_uses_current_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            previous_cwd = Path.cwd()
            try:
                import os

                os.chdir(tmp)
                expected_path = Path.cwd() / DEFAULT_HISTORY_DB_NAME
                db = HistoryDB()
                self.assertEqual(db.path, expected_path)
                self.assertTrue(db.path.exists())
            finally:
                os.chdir(previous_cwd)

    def test_schema_idempotent(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "h.db"
            HistoryDB(path)
            HistoryDB(path)  # 二次实例化不应报错

    def test_insert_list_get(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            db = HistoryDB(Path(tmp) / "h.db")
            run_id = db.insert_run(
                quests_dir="/packs/ATM10/config/ftbquests/quests",
                mode="lang",
                model="deepseek-v4-flash",
                style="自然",
                base_url="https://api.deepseek.com",
                total_entries=2,
                translated_entries=2,
                cache_hits=0,
                failed_count=0,
                warning_count=0,
                files=_sample_files(),
            )
            self.assertGreater(run_id, 0)

            runs = db.list_runs()
            self.assertEqual(len(runs), 1)
            self.assertEqual(runs[0].id, run_id)
            self.assertEqual(runs[0].pack_name, "ATM10")
            self.assertEqual(runs[0].mode, "lang")

            files = db.get_files(run_id)
            self.assertEqual(len(files), 1)
            self.assertEqual(files[0].filename, "zh_cn.snbt")
            self.assertEqual(files[0].mapping["quest.1.title"]["zh"], "第一个任务")

    def test_delete_cascades(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            db = HistoryDB(Path(tmp) / "h.db")
            run_id = db.insert_run(
                quests_dir="/x", mode="lang", model="m", style="s", base_url="u",
                total_entries=1, translated_entries=1, cache_hits=0,
                failed_count=0, warning_count=0,
                files=_sample_files(),
            )
            self.assertEqual(len(db.get_files(run_id)), 1)
            db.delete_run(run_id)
            self.assertIsNone(db.get_run(run_id))
            self.assertEqual(db.get_files(run_id), [])

    def test_export_zip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            db = HistoryDB(Path(tmp) / "h.db")
            run_id = db.insert_run(
                quests_dir="/packs/Pack/config/ftbquests/quests",
                mode="lang", model="deepseek-v4-flash", style="自然",
                base_url="https://api.deepseek.com",
                total_entries=2, translated_entries=2, cache_hits=0,
                failed_count=0, warning_count=0,
                files=_sample_files(),
            )
            dest = Path(tmp) / "out.zip"
            written = db.export_zip(run_id, dest)
            self.assertTrue(written.exists())

            with zipfile.ZipFile(written) as zf:
                names = set(zf.namelist())
                self.assertIn("manifest.json", names)
                self.assertIn("lang/zh_cn.snbt", names)
                manifest = json.loads(zf.read("manifest.json"))
                self.assertEqual(manifest["pack_name"], "Pack")
                self.assertEqual(manifest["mode"], "lang")
                self.assertEqual(manifest["files"], ["lang/zh_cn.snbt"])
                self.assertIn("第一个任务", zf.read("lang/zh_cn.snbt").decode("utf-8"))

    def test_export_zip_preserves_chapters_subdir_structure(self) -> None:
        """chapters 模式的 ZIP 也应保留 chapters/ 前缀。"""
        with tempfile.TemporaryDirectory() as tmp:
            db = HistoryDB(Path(tmp) / "h.db")
            rid = db.insert_run(
                quests_dir="/packs/Pack/config/ftbquests/quests",
                mode="chapters", model="m", style="s", base_url="u",
                total_entries=2, translated_entries=2, cache_hits=0,
                failed_count=0, warning_count=0,
                files=[
                    FileRecord(filename="chapters/intro.snbt", mapping={}, output_content="{intro}"),
                    FileRecord(filename="chapters/main.snbt", mapping={}, output_content="{main}"),
                ],
            )
            dest = Path(tmp) / "out.zip"
            db.export_zip(rid, dest)
            with zipfile.ZipFile(dest) as zf:
                names = set(zf.namelist())
                self.assertIn("chapters/intro.snbt", names)
                self.assertIn("chapters/main.snbt", names)

    def test_export_zip_rejects_unsafe_paths(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            db = HistoryDB(Path(tmp) / "h.db")
            for filename in ("../evil.snbt", r"C:\tmp\evil.snbt"):
                rid = db.insert_run(
                    quests_dir="/packs/Pack/config/ftbquests/quests",
                    mode="chapters", model="m", style="s", base_url="u",
                    total_entries=1, translated_entries=1, cache_hits=0,
                    failed_count=0, warning_count=0,
                    files=[FileRecord(
                        filename=filename,
                        mapping={"k": {"en": "Hi", "zh": "你好"}},
                        output_content='{"k": "你好"}',
                    )],
                )

                with self.assertRaises(ValueError):
                    db.export_zip(rid, Path(tmp) / f"{rid}.zip")

    def test_pack_name_derivation(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            db = HistoryDB(Path(tmp) / "h.db")
            rid = db.insert_run(
                quests_dir="/foo/bar/MyPack/config/ftbquests/quests",
                mode="lang", model="m", style="s", base_url="u",
                total_entries=1, translated_entries=1, cache_hits=0,
                failed_count=0, warning_count=0, files=_sample_files(),
            )
            self.assertEqual(db.get_run(rid).pack_name, "MyPack")


if __name__ == "__main__":
    unittest.main()
