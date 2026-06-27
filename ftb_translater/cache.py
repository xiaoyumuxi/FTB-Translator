from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

from ftb_translater.logger import get_logger

_log = get_logger(__name__)


class TranslationCache:
    def __init__(self, path: Path):
        self.path = path
        self._data: dict[str, str] = {}

    def load(self) -> None:
        if not self.path.exists():
            _log.debug("Cache file not found, starting empty: %s", self.path)
            self._data = {}
            return
        _log.debug("Loading cache from %s", self.path)
        with self.path.open("r", encoding="utf-8") as file:
            raw = json.load(file)
        if not isinstance(raw, dict):
            _log.error("Invalid cache file (not a JSON object): %s", self.path)
            raise ValueError(f"Invalid cache file: {self.path}")
        self._data = {str(key): str(value) for key, value in raw.items()}
        _log.debug("Cache loaded: %d entries", len(self._data))

    def save(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        _log.debug("Saving cache to %s (%d entries)", self.path, len(self._data))
        with self.path.open("w", encoding="utf-8") as file:
            json.dump(self._data, file, ensure_ascii=False, indent=2, sort_keys=True)

    def get(self, source_text: str, model: str, target_locale: str, style: str) -> str | None:
        result = self._data.get(self._key(source_text, model, target_locale, style))
        if result is not None:
            _log.debug("Cache hit for text (len=%d)", len(source_text))
        return result

    def set(self, source_text: str, model: str, target_locale: str, style: str, translation: str) -> None:
        self._data[self._key(source_text, model, target_locale, style)] = translation

    @staticmethod
    def _key(source_text: str, model: str, target_locale: str, style: str) -> str:
        payload: dict[str, Any] = {
            "source_text": source_text,
            "model": model,
            "target_locale": target_locale,
            "style": style,
        }
        encoded = json.dumps(payload, ensure_ascii=False, sort_keys=True).encode("utf-8")
        return hashlib.sha256(encoded).hexdigest()
