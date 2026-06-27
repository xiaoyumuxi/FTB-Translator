from __future__ import annotations

import shutil
from datetime import datetime
from pathlib import Path

from ftb_translater.logger import get_logger

_log = get_logger(__name__)


def create_backup(quests_dir: Path, directories: tuple[str, ...] = ("lang",)) -> Path:
    timestamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    backup_root = quests_dir / ".ftb-translater" / "backups" / timestamp
    copied = False
    for directory in directories:
        source = quests_dir / directory
        if not source.is_dir():
            _log.warning("Backup source not found, skipping: %s", source)
            continue
        destination = backup_root / directory
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(source, destination)
        _log.info("Backed up %s -> %s", source, destination)
        copied = True
    if not copied:
        names = ", ".join(directories)
        _log.error("No backup source directories found under %s: %s", quests_dir, names)
        raise FileNotFoundError(f"Missing backup source directories under {quests_dir}: {names}")
    return backup_root
