from __future__ import annotations

import json
import time
from collections.abc import Callable, Mapping
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from ftb_translater.logger import get_logger


DEFAULT_DEEPL_BASE_URL = "https://api-free.deepl.com"
DEFAULT_DEEPL_MODEL = "deepl"

_log = get_logger(__name__)


class DeepLTranslationError(RuntimeError):
    pass


class DeepLTranslator:
    """Adapter for DeepL's text translation API.

    Keys are preserved locally: only values are sent to DeepL, then results are
    paired back to their original keys by position.
    """

    def __init__(
        self,
        api_key: str,
        model: str = DEFAULT_DEEPL_MODEL,
        base_url: str = DEFAULT_DEEPL_BASE_URL,
        retries: int = 2,
        timeout: float = 60,
        logger: Callable[[str], None] | None = None,
    ):
        if not api_key.strip():
            raise ValueError("DeepL API Key is required.")
        self.api_key = api_key.strip()
        self.model = model or DEFAULT_DEEPL_MODEL
        self.base_url = base_url.rstrip("/")
        self.retries = retries
        self.timeout = timeout
        self.logger = logger

    def translate_batch(self, entries: Mapping[str, str], style: str = "") -> dict[str, str]:
        if not entries:
            return {}
        keys = list(entries)
        texts = [entries[key] for key in keys]
        last_error: Exception | None = None
        for attempt in range(self.retries + 1):
            try:
                msg = f"Calling DeepL: {len(texts)} entries, attempt {attempt + 1}."
                _log.info(msg)
                self._log(msg)
                translated = self._request(texts)
                return dict(zip(keys, translated, strict=True))
            except Exception as exc:  # noqa: BLE001
                last_error = exc
                msg = f"DeepL batch attempt {attempt + 1} failed: {exc}"
                _log.warning(msg)
                self._log(msg)
                if attempt < self.retries:
                    time.sleep(0.8 * (attempt + 1))
        raise DeepLTranslationError(f"DeepL translation failed: {last_error}") from last_error

    def _request(self, texts: list[str]) -> list[str]:
        payload = json.dumps(
            {"text": texts, "source_lang": "EN", "target_lang": "ZH-HANS"},
            ensure_ascii=False,
        ).encode("utf-8")
        request = Request(
            f"{self.base_url}/v2/translate",
            data=payload,
            headers={
                "Authorization": f"DeepL-Auth-Key {self.api_key}",
                "Content-Type": "application/json",
                "User-Agent": "FTB-Translater/0.1",
            },
            method="POST",
        )
        try:
            with urlopen(request, timeout=self.timeout) as response:  # noqa: S310 - URL is user-configurable by design.
                raw = json.loads(response.read().decode("utf-8"))
        except HTTPError as exc:
            detail = exc.read().decode("utf-8", errors="replace")[:500]
            raise DeepLTranslationError(f"DeepL HTTP {exc.code}: {detail}") from exc
        except (URLError, TimeoutError, json.JSONDecodeError) as exc:
            raise DeepLTranslationError(f"DeepL request failed: {exc}") from exc

        translations = raw.get("translations") if isinstance(raw, dict) else None
        if not isinstance(translations, list) or len(translations) != len(texts):
            raise DeepLTranslationError("DeepL returned an unexpected number of translations.")
        result: list[str] = []
        for item in translations:
            if not isinstance(item, dict) or not isinstance(item.get("text"), str):
                raise DeepLTranslationError("DeepL returned an invalid translation item.")
            result.append(item["text"])
        return result

    def _log(self, message: str) -> None:
        if self.logger:
            self.logger(message)
