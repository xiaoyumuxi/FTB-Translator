from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


@dataclass
class TranslationReport:
    source_file: str
    target_file: str
    backup_dir: str
    total_entries: int
    translated_entries: int
    cache_hits: int
    failed_entries: list[str] = field(default_factory=list)
    warnings: dict[str, list[str]] = field(default_factory=dict)
    failed_translations: dict[str, dict[str, str]] = field(default_factory=dict)

    def save(self, quests_dir: Path) -> Path:
        path = quests_dir / ".ftb-translater" / "report-latest.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("w", encoding="utf-8") as file:
            json.dump(self.to_dict(), file, ensure_ascii=False, indent=2)
        return path

    def to_dict(self) -> dict[str, Any]:
        return {
            "source_file": self.source_file,
            "target_file": self.target_file,
            "backup_dir": self.backup_dir,
            "total_entries": self.total_entries,
            "translated_entries": self.translated_entries,
            "cache_hits": self.cache_hits,
            "failed_entries": self.failed_entries,
            "warnings": self.warnings,
            "failed_translations": self.failed_translations,
        }
