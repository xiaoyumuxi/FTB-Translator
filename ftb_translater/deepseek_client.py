from __future__ import annotations

import json
import time
from collections.abc import Callable, Mapping
from typing import Any

from ftb_translater.logger import get_logger

DEFAULT_BASE_URL = "https://api.deepseek.com"
DEFAULT_MODEL = "deepseek-v4-flash"
DEFAULT_STYLE = "自然玩家向简体中文汉化"

_log = get_logger(__name__)


class DeepSeekTranslationError(RuntimeError):
    pass


class DeepSeekTranslator:
    def __init__(
        self,
        api_key: str,
        model: str = DEFAULT_MODEL,
        base_url: str = DEFAULT_BASE_URL,
        client: Any | None = None,
        retries: int = 2,
        logger: Callable[[str], None] | None = None,
    ):
        if not api_key.strip() and client is None:
            raise ValueError("DeepSeek API Key is required.")
        self.model = model
        self.retries = retries
        self.logger = logger
        self.client = client or self._create_client(api_key=api_key, base_url=base_url)

    def translate_batch(self, entries: Mapping[str, str], style: str = DEFAULT_STYLE) -> dict[str, str]:
        if not entries:
            return {}
        prompt = self._build_prompt(entries, style)
        last_error: Exception | None = None
        for attempt in range(self.retries + 1):
            try:
                msg = f"Calling DeepSeek {self.model}: {len(entries)} entries, attempt {attempt + 1}."
                _log.info(msg)
                self._log(msg)
                return self._request_json(prompt, expected_keys=set(entries))
            except Exception as exc:  # noqa: BLE001
                last_error = exc
                msg = f"DeepSeek batch attempt {attempt + 1} failed: {exc}"
                _log.warning(msg)
                self._log(msg)
                if attempt < self.retries:
                    time.sleep(0.8 * (attempt + 1))

        recovered: dict[str, str] = {}
        failures: list[str] = []
        for key, value in entries.items():
            try:
                msg = f"Retrying DeepSeek as single entry: {key}"
                _log.info(msg)
                self._log(msg)
                recovered[key] = self._request_json(
                    self._build_prompt({key: value}, style),
                    expected_keys={key},
                )[key]
            except Exception as exc:  # noqa: BLE001
                _log.error("Single-entry retry failed for key %r: %s", key, exc)
                failures.append(f"{key}: {exc}")
        if failures:
            message = "; ".join(failures)
            if last_error:
                message = f"batch failed with {last_error}; single-item failures: {message}"
            raise DeepSeekTranslationError(message)
        return recovered

    def _request_json(self, prompt: str, expected_keys: set[str]) -> dict[str, str]:
        _log.debug("Sending request to DeepSeek, expected keys: %s", sorted(expected_keys))
        response = self.client.chat.completions.create(
            model=self.model,
            messages=[
                {
                    "role": "system",
                    "content": (
                        "You are a Minecraft modpack localization assistant. "
                        "Translate only user-facing English quest text into Simplified Chinese."
                    ),
                },
                {"role": "user", "content": prompt},
            ],
            temperature=0.2,
            response_format={"type": "json_object"},
        )
        content = response.choices[0].message.content
        if not content:
            raise DeepSeekTranslationError("DeepSeek returned an empty response.")
        _log.debug("DeepSeek raw response length: %d chars", len(content))
        try:
            raw = json.loads(content)
        except json.JSONDecodeError as exc:
            _log.error("DeepSeek returned invalid JSON: %s\nRaw content: %s", exc, content[:500])
            raise DeepSeekTranslationError(f"DeepSeek returned invalid JSON: {exc}") from exc

        if not isinstance(raw, dict):
            _log.error("DeepSeek JSON response is not a dict, got %s: %s", type(raw).__name__, str(raw)[:200])
            raise DeepSeekTranslationError("DeepSeek JSON response must be an object.")

        missing = expected_keys - set(raw)
        if missing:
            _log.error("DeepSeek response missed keys: %s. Got keys: %s", sorted(missing), sorted(raw.keys()))
            raise DeepSeekTranslationError(f"DeepSeek response missed keys: {sorted(missing)}")

        _log.debug("DeepSeek response OK: %d keys returned", len(expected_keys))
        return {key: str(raw[key]) for key in expected_keys}

    @staticmethod
    def _build_prompt(entries: Mapping[str, str], style: str) -> str:
        payload = json.dumps(entries, ensure_ascii=False, indent=2)
        return (
            f"Translate this FTB Quests language map to Simplified Chinese.\n"
            f"Style: {style}.\n"
            "Return one JSON object with exactly the same keys and translated string values.\n"
            "Preserve all Minecraft formatting codes, placeholders, item IDs, tags, markdown links, "
            "line breaks, escape sequences, and numbers. Do not translate keys.\n\n"
            f"{payload}"
        )

    @staticmethod
    def _create_client(api_key: str, base_url: str) -> Any:
        try:
            from openai import OpenAI
        except ImportError as exc:
            raise DeepSeekTranslationError(
                "Missing dependency 'openai'. Run `python -m pip install -e .` before using DeepSeek translation."
            ) from exc
        _log.debug("Creating OpenAI client with base_url=%s", base_url)
        return OpenAI(api_key=api_key.strip(), base_url=base_url)

    def _log(self, message: str) -> None:
        if self.logger:
            self.logger(message)
