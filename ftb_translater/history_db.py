"""翻译历史 SQLite 存储。

每次翻译完成后保存一条 run + 对应文件的完整中英映射 + 输出文件原文。
支持列表查询、删除、ZIP 导出,后续可用于词表挖掘。
"""
from __future__ import annotations

import json
import sqlite3
import zipfile
from contextlib import contextmanager
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path, PurePosixPath
from typing import Iterator

from ftb_translater.logger import get_logger
_log = get_logger(__name__)
DEFAULT_HISTORY_DB_NAME = "history.sqlite3"


_SCHEMA = """
CREATE TABLE IF NOT EXISTS translation_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    quests_dir TEXT NOT NULL,
    pack_name TEXT,
    mode TEXT NOT NULL,
    model TEXT NOT NULL,
    style TEXT NOT NULL,
    base_url TEXT,
    total_entries INTEGER NOT NULL,
    translated_entries INTEGER NOT NULL,
    cache_hits INTEGER NOT NULL,
    failed_count INTEGER NOT NULL,
    warning_count INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS translation_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL,
    filename TEXT NOT NULL,
    source_hash TEXT,
    mapping TEXT NOT NULL,
    output_content TEXT NOT NULL,
    FOREIGN KEY(run_id) REFERENCES translation_runs(id) ON DELETE CASCADE,
    UNIQUE(run_id, filename)
);

CREATE INDEX IF NOT EXISTS idx_runs_created ON translation_runs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_files_run ON translation_files(run_id);
"""


@dataclass
class FileRecord:
    filename: str
    mapping: dict[str, dict[str, str]]
    output_content: str
    source_hash: str = ""


@dataclass
class RunSummary:
    id: int
    pack_name: str
    quests_dir: str
    mode: str
    model: str
    style: str
    total_entries: int
    translated_entries: int
    cache_hits: int
    failed_count: int
    warning_count: int
    created_at: str


@dataclass
class RunDetail:
    summary: RunSummary
    files: list[FileRecord] = field(default_factory=list)


class HistoryDB:
    def __init__(self, path: Path | None = None):
        self.path = path or (Path.cwd() / DEFAULT_HISTORY_DB_NAME)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.init_schema()

    @contextmanager
    def _connect(self) -> Iterator[sqlite3.Connection]:
        conn = sqlite3.connect(self.path)
        conn.row_factory = sqlite3.Row
        try:
            conn.execute("PRAGMA foreign_keys = ON")
            yield conn
            conn.commit()
        except Exception:
            conn.rollback()
            raise
        finally:
            conn.close()

    def init_schema(self) -> None:
        with self._connect() as conn:
            conn.executescript(_SCHEMA)

    def insert_run(
        self,
        *,
        quests_dir: str,
        mode: str,
        model: str,
        style: str,
        base_url: str,
        total_entries: int,
        translated_entries: int,
        cache_hits: int,
        failed_count: int,
        warning_count: int,
        files: list[FileRecord],
        pack_name: str | None = None,
        created_at: str | None = None,
    ) -> int:
        pack_name = pack_name or _derive_pack_name(quests_dir)
        created_at = created_at or datetime.now().isoformat(timespec="seconds")
        with self._connect() as conn:
            cursor = conn.execute(
                """
                INSERT INTO translation_runs(
                    quests_dir, pack_name, mode, model, style, base_url,
                    total_entries, translated_entries, cache_hits,
                    failed_count, warning_count, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    quests_dir, pack_name, mode, model, style, base_url,
                    total_entries, translated_entries, cache_hits,
                    failed_count, warning_count, created_at,
                ),
            )
            run_id = cursor.lastrowid
            assert run_id is not None
            for record in files:
                conn.execute(
                    """
                    INSERT INTO translation_files(
                        run_id, filename, source_hash, mapping, output_content
                    ) VALUES (?, ?, ?, ?, ?)
                    """,
                    (
                        run_id,
                        record.filename,
                        record.source_hash,
                        json.dumps(record.mapping, ensure_ascii=False),
                        record.output_content,
                    ),
                )
            _log.info("Inserted history run #%d with %d file(s)", run_id, len(files))
            return run_id

    def list_runs(self, limit: int = 100) -> list[RunSummary]:
        with self._connect() as conn:
            rows = conn.execute(
                """
                SELECT id, pack_name, quests_dir, mode, model, style,
                       total_entries, translated_entries, cache_hits,
                       failed_count, warning_count, created_at
                FROM translation_runs
                ORDER BY created_at DESC, id DESC
                LIMIT ?
                """,
                (limit,),
            ).fetchall()
            return [_row_to_summary(row) for row in rows]

    def get_run(self, run_id: int) -> RunSummary | None:
        with self._connect() as conn:
            row = conn.execute(
                """
                SELECT id, pack_name, quests_dir, mode, model, style,
                       total_entries, translated_entries, cache_hits,
                       failed_count, warning_count, created_at
                FROM translation_runs WHERE id = ?
                """,
                (run_id,),
            ).fetchone()
            return _row_to_summary(row) if row else None

    def get_files(self, run_id: int) -> list[FileRecord]:
        with self._connect() as conn:
            rows = conn.execute(
                """
                SELECT filename, source_hash, mapping, output_content
                FROM translation_files WHERE run_id = ?
                ORDER BY filename
                """,
                (run_id,),
            ).fetchall()
            return [
                FileRecord(
                    filename=row["filename"],
                    source_hash=row["source_hash"] or "",
                    mapping=json.loads(row["mapping"]),
                    output_content=row["output_content"],
                )
                for row in rows
            ]

    def delete_run(self, run_id: int) -> None:
        with self._connect() as conn:
            conn.execute("DELETE FROM translation_files WHERE run_id = ?", (run_id,))
            conn.execute("DELETE FROM translation_runs WHERE id = ?", (run_id,))
            _log.info("Deleted history run #%d", run_id)

    def export_zip(self, run_id: int, dest: Path) -> Path:
        summary = self.get_run(run_id)
        if summary is None:
            raise ValueError(f"Run #{run_id} not found")
        files = self.get_files(run_id)
        if not files:
            raise ValueError(f"Run #{run_id} has no files")

        dest = Path(dest)
        dest.parent.mkdir(parents=True, exist_ok=True)
        archive_files = [_archive_name(record.filename, summary.mode) for record in files]
        manifest = {
            "run_id": summary.id,
            "pack_name": summary.pack_name,
            "quests_dir": summary.quests_dir,
            "mode": summary.mode,
            "model": summary.model,
            "style": summary.style,
            "total_entries": summary.total_entries,
            "translated_entries": summary.translated_entries,
            "failed_count": summary.failed_count,
            "warning_count": summary.warning_count,
            "created_at": summary.created_at,
            "files": archive_files,
        }
        with zipfile.ZipFile(dest, "w", zipfile.ZIP_DEFLATED) as zf:
            zf.writestr("manifest.json", json.dumps(manifest, ensure_ascii=False, indent=2))
            for record, archive_name in zip(files, archive_files, strict=True):
                zf.writestr(archive_name, record.output_content)
        _log.info("Exported run #%d to %s", run_id, dest)
        return dest


def _row_to_summary(row: sqlite3.Row) -> RunSummary:
    return RunSummary(
        id=row["id"],
        pack_name=row["pack_name"] or "",
        quests_dir=row["quests_dir"],
        mode=row["mode"],
        model=row["model"],
        style=row["style"],
        total_entries=row["total_entries"],
        translated_entries=row["translated_entries"],
        cache_hits=row["cache_hits"],
        failed_count=row["failed_count"],
        warning_count=row["warning_count"],
        created_at=row["created_at"],
    )


def _derive_pack_name(quests_dir: str) -> str:
    path = Path(quests_dir)
    parts = path.parts
    for marker in ("config", "ftbquests"):
        if marker in parts:
            idx = parts.index(marker)
            if idx > 0:
                return parts[idx - 1]
    return path.name or "unknown"


def _archive_name(filename: str, mode: str) -> str:
    normalized = filename.replace("\\", "/").strip("/")
    if mode == "lang" and normalized == "zh_cn.snbt":
        normalized = "lang/zh_cn.snbt"

    parts = PurePosixPath(normalized).parts
    if (
        not normalized
        or PurePosixPath(normalized).is_absolute()
        or ".." in parts
        or (parts and parts[0].endswith(":"))
        or normalized.startswith("/")
    ):
        raise ValueError(f"Unsafe history export path: {filename!r}")

    if mode == "lang":
        allowed = normalized == "lang/zh_cn.snbt"
    else:
        allowed = (
            normalized.startswith("chapters/")
            and len(parts) == 2
            and parts[1].endswith(".snbt")
        )
    if not allowed:
        raise ValueError(f"Unexpected history export path for {mode} mode: {filename!r}")
    return normalized
