from __future__ import annotations

import logging
import logging.handlers
from pathlib import Path


LOG_FORMAT = "%(asctime)s [%(levelname)s] %(name)s: %(message)s"
DATE_FORMAT = "%Y-%m-%d %H:%M:%S"

_initialized = False


def setup_logging(log_dir: Path | None = None, level: int = logging.DEBUG) -> Path:
    """Configure file + console logging. Call once at startup. Returns the log file path."""
    global _initialized
    if _initialized:
        return _get_log_path(log_dir)

    log_path = _get_log_path(log_dir)
    log_path.parent.mkdir(parents=True, exist_ok=True)

    root = logging.getLogger()
    root.setLevel(level)

    # Rotating file handler — keep 5 files of 1 MB each
    file_handler = logging.handlers.RotatingFileHandler(
        log_path,
        maxBytes=1024 * 1024,
        backupCount=5,
        encoding="utf-8",
    )
    file_handler.setLevel(logging.DEBUG)
    file_handler.setFormatter(logging.Formatter(LOG_FORMAT, DATE_FORMAT))
    root.addHandler(file_handler)

    # Console handler shows INFO and above
    console_handler = logging.StreamHandler()
    console_handler.setLevel(logging.INFO)
    console_handler.setFormatter(logging.Formatter(LOG_FORMAT, DATE_FORMAT))
    root.addHandler(console_handler)

    _initialized = True
    logging.getLogger(__name__).debug("Logging initialised. Log file: %s", log_path)
    return log_path


def get_logger(name: str) -> logging.Logger:
    return logging.getLogger(name)


def _get_log_path(log_dir: Path | None) -> Path:
    base = log_dir or Path.cwd() / ".ftb-translater" / "logs"
    return base / "ftb_translater.log"
