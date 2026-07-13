from __future__ import annotations

import json
import time
import uuid
from collections.abc import Callable, Mapping
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen

from ftb_translater.logger import get_logger


DEFAULT_GOOGLE_WEB_BASE_URL = "https://translate.googleapis.com"
DEFAULT_DEEPL_WEB_BASE_URL = "https://oneshot-free.www.deepl.com"
GOOGLE_WEB_MODEL = "google-web"
DEEPL_WEB_MODEL = "deepl-web"
DEEPL_WEB_MAX_CHARS = 1500
GOOGLE_WEB_MAX_CHARS = 4500

_log = get_logger(__name__)


class WebTranslationError(RuntimeError):
    pass


class GoogleWebTranslator:
    """Experimental adapter for Google Translate's undocumented web endpoint."""

    def __init__(
        self,
        api_key: str = "",
        model: str = GOOGLE_WEB_MODEL,
        base_url: str = DEFAULT_GOOGLE_WEB_BASE_URL,
        retries: int = 2,
        timeout: float = 30,
        logger: Callable[[str], None] | None = None,
    ):
        self.model = model or GOOGLE_WEB_MODEL
        self.base_url = base_url.rstrip("/")
        self.retries = retries
        self.timeout = timeout
        self.logger = logger

    def translate_batch(self, entries: Mapping[str, str], style: str = "") -> dict[str, str]:
        if not entries:
            return {}
        units: list[tuple[str, str]] = []
        piece_limit = GOOGLE_WEB_MAX_CHARS - 100
        for key, text in entries.items():
            units.extend((key, piece) for piece in _split_text(text, piece_limit))

        result = {key: "" for key in entries}
        chunk: list[tuple[str, str]] = []
        chunk_chars = 0
        for unit in units:
            estimated_chars = len(unit[1]) + 40
            if chunk and chunk_chars + estimated_chars > GOOGLE_WEB_MAX_CHARS:
                self._append_google_results(result, chunk)
                chunk = []
                chunk_chars = 0
            chunk.append(unit)
            chunk_chars += estimated_chars
        if chunk:
            self._append_google_results(result, chunk)
        return result

    def _append_google_results(self, result: dict[str, str], units: list[tuple[str, str]]) -> None:
        translated = self._translate_chunk([text for _key, text in units])
        for (key, _source), text in zip(units, translated, strict=True):
            result[key] += text

    def _translate_chunk(self, texts: list[str]) -> list[str]:
        markers = [f"⟪FTB_TRANSLATER_BATCH_{index}⟫" for index in range(len(texts))]
        if any(marker in text for marker in markers for text in texts):
            raise WebTranslationError("Source text contains a reserved Google batch marker.")
        combined = "\n".join(marker + text for marker, text in zip(markers, texts, strict=True))
        body = urlencode(
            {"client": "gtx", "sl": "en", "tl": "zh-CN", "dt": "t", "q": combined}
        ).encode("utf-8")
        request = Request(
            f"{self.base_url}/translate_a/single",
            data=body,
            headers={
                "Content-Type": "application/x-www-form-urlencoded",
                "User-Agent": _USER_AGENT,
            },
            method="POST",
        )
        raw = self._request_with_retry(request)
        try:
            payload = json.loads(raw)
            segments = payload[0]
            translated = "".join(segment[0] for segment in segments if segment and isinstance(segment[0], str))
        except (json.JSONDecodeError, IndexError, TypeError) as exc:
            raise WebTranslationError(f"Google web translation returned invalid JSON: {exc}") from exc
        if not translated:
            raise WebTranslationError("Google web translation returned empty text.")
        positions = [translated.find(marker) for marker in markers]
        if any(position < 0 for position in positions) or positions != sorted(positions):
            raise WebTranslationError("Google web translation did not preserve batch markers.")
        results: list[str] = []
        for index, marker in enumerate(markers):
            start = positions[index] + len(marker)
            end = positions[index + 1] if index + 1 < len(markers) else len(translated)
            value = translated[start:end]
            if index + 1 < len(markers) and value.endswith("\n"):
                value = value[:-1]
            results.append(value)
        return results

    def _request_with_retry(self, request: Request) -> str:
        last_error: Exception | None = None
        for attempt in range(self.retries + 1):
            try:
                with urlopen(request, timeout=self.timeout) as response:  # noqa: S310 - endpoint is configurable.
                    return response.read().decode("utf-8")
            except (HTTPError, URLError, TimeoutError) as exc:
                last_error = exc
                self._log(f"Google web request attempt {attempt + 1} failed: {exc}")
                if attempt < self.retries:
                    time.sleep(1.2 * (attempt + 1))
        raise WebTranslationError(f"Google web translation failed: {last_error}") from last_error

    def _log(self, message: str) -> None:
        _log.warning(message)
        if self.logger:
            self.logger(message)


class DeepLWebTranslator:
    """Experimental adapter matching DeepL's anonymous browser-extension request."""

    def __init__(
        self,
        api_key: str = "",
        model: str = DEEPL_WEB_MODEL,
        base_url: str = DEFAULT_DEEPL_WEB_BASE_URL,
        retries: int = 2,
        timeout: float = 30,
        logger: Callable[[str], None] | None = None,
    ):
        self.model = model or DEEPL_WEB_MODEL
        self.base_url = base_url.rstrip("/")
        self.retries = retries
        self.timeout = timeout
        self.logger = logger
        self.instance_id = str(uuid.uuid4())

    def translate_batch(self, entries: Mapping[str, str], style: str = "") -> dict[str, str]:
        if not entries:
            return {}
        result: dict[str, str] = {}
        chunk: list[tuple[str, str]] = []
        chunk_chars = 0
        for key, text in entries.items():
            text_chars = len(text)
            if text_chars > DEEPL_WEB_MAX_CHARS:
                if chunk:
                    result.update(self._translate_chunk(chunk))
                    chunk = []
                    chunk_chars = 0
                result[key] = "".join(self._translate_texts(_split_text(text, DEEPL_WEB_MAX_CHARS)))
                continue
            if chunk and chunk_chars + text_chars > DEEPL_WEB_MAX_CHARS:
                result.update(self._translate_chunk(chunk))
                chunk = []
                chunk_chars = 0
            chunk.append((key, text))
            chunk_chars += text_chars
        if chunk:
            result.update(self._translate_chunk(chunk))
        return result

    def _translate_chunk(self, items: list[tuple[str, str]]) -> dict[str, str]:
        values = self._translate_texts([text for _key, text in items])
        return {key: value for (key, _text), value in zip(items, values, strict=True)}

    def _translate_texts(self, texts: list[str]) -> list[str]:
        payload = {
            "text": texts,
            "target_lang": "zh-Hans",
            "source_lang": "en",
            "usage_type": "Translate",
            "app_information": {
                "os": "brex_macOS",
                "os_version": "brex_chrome_120.0.0.0",
                "app_version": "1.86.0",
                "app_build": "chrome_web_store",
                "instance_id": self.instance_id,
            },
        }
        request = Request(
            f"{self.base_url}/v1/translate",
            data=json.dumps(payload, ensure_ascii=False).encode("utf-8"),
            headers={
                "Content-Type": "application/json",
                "Accept": "*/*",
                "Authorization": "None",
                "Origin": "chrome-extension://cofdbpoegempjloogbagkncekinflcnj",
                "Sec-Fetch-Site": "cross-site",
                "Sec-Fetch-Mode": "cors",
                "Sec-Fetch-Dest": "empty",
                "User-Agent": _USER_AGENT,
            },
            method="POST",
        )
        raw = self._request_with_retry(request)
        try:
            response = json.loads(raw)
            translations = response["translations"]
            values = [item["text"] for item in translations]
        except (json.JSONDecodeError, KeyError, TypeError) as exc:
            raise WebTranslationError(f"DeepL web translation returned invalid JSON: {exc}") from exc
        if len(values) != len(texts) or not all(isinstance(value, str) for value in values):
            raise WebTranslationError("DeepL web translation returned an unexpected number of results.")
        return values

    def _request_with_retry(self, request: Request) -> str:
        last_error: Exception | None = None
        for attempt in range(self.retries + 1):
            try:
                with urlopen(request, timeout=self.timeout) as response:  # noqa: S310 - endpoint is configurable.
                    return response.read().decode("utf-8")
            except (HTTPError, URLError, TimeoutError) as exc:
                last_error = exc
                self._log(f"DeepL web request attempt {attempt + 1} failed: {exc}")
                if attempt < self.retries:
                    time.sleep(1.5 * (attempt + 1))
        raise WebTranslationError(f"DeepL web translation failed: {last_error}") from last_error

    def _log(self, message: str) -> None:
        _log.warning(message)
        if self.logger:
            self.logger(message)


_USER_AGENT = (
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
    "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
)


def _split_text(text: str, max_chars: int) -> list[str]:
    """Split long text without cutting an opaque ⟨P_n⟩ placeholder."""
    chunks: list[str] = []
    start = 0
    while len(text) - start > max_chars:
        end = start + max_chars
        window = text[start:end]
        cut = max((window.rfind(char) + 1 for char in "\n.!?。！？;； " ), default=0)
        if cut < max_chars // 2:
            cut = max_chars
        open_pos = window.rfind("⟨")
        close_pos = window.rfind("⟩")
        if open_pos > close_pos and open_pos < cut:
            cut = open_pos
        if cut <= 0:
            cut = max_chars
        chunks.append(text[start : start + cut])
        start += cut
    if start < len(text):
        chunks.append(text[start:])
    return chunks
