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


class OpenAICompatibleTranslator:
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
                msg = f"Calling OpenAI-compatible API {self.model}: {len(entries)} entries, attempt {attempt + 1}."
                _log.info(msg)
                self._log(msg)
                return self._request_json(prompt, expected_keys=set(entries))
            except Exception as exc:  # noqa: BLE001
                last_error = exc
                msg = f"OpenAI-compatible batch attempt {attempt + 1} failed: {exc}"
                _log.warning(msg)
                self._log(msg)
                if attempt < self.retries:
                    time.sleep(0.8 * (attempt + 1))

        recovered: dict[str, str] = {}
        failures: list[str] = []
        for key, value in entries.items():
            try:
                msg = f"Retrying OpenAI-compatible API as single entry: {key}"
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
        response = self._request_json_response(prompt)

        content = response.choices[0].message.content
        if not content:
            raise DeepSeekTranslationError("DeepSeek returned an empty response.")
        _log.debug("DeepSeek raw response length: %d chars", len(content))
        try:
            raw = self._parse_json_object(content)
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

        extra = set(raw) - expected_keys
        if extra:
            _log.warning("DeepSeek response returned extra keys that will be ignored: %s", sorted(extra))

        _log.debug("DeepSeek response OK: %d keys returned", len(expected_keys))
        return {key: str(raw[key]) for key in expected_keys}

    def _request_json_response(self, prompt: str) -> Any:
        request = {
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": (
                        "You are a Minecraft modpack localization assistant. 你是 Minecraft 整合包任务书汉化助手。 "
                        "Translate only user-facing English quest text into natural Simplified Chinese. "
                        "只翻译玩家可见英文为自然简体中文。 "
                        "Never alter, remove, merge, or invent opaque placeholders such as ⟨P_0⟩. "
                        "绝不能修改、删除、合并或新增 ⟨P_0⟩ 这类占位符。"
                    ),
                },
                {"role": "user", "content": prompt},
            ],
            "temperature": 0.2,
        }
        try:
            return self.client.chat.completions.create(
                **request,
                response_format={"type": "json_object"},
            )
        except Exception as exc:  # noqa: BLE001
            if not self._is_response_format_unsupported(exc):
                raise
            msg = "API does not support response_format=json_object; retrying with prompt-only JSON mode."
            _log.info(msg)
            self._log(msg)
            return self.client.chat.completions.create(**request)

    @staticmethod
    def _parse_json_object(content: str) -> Any:
        text = content.strip()
        if text.startswith("```"):
            lines = text.splitlines()
            if lines and lines[0].strip().lower() in {"```", "```json"}:
                lines = lines[1:]
            if lines and lines[-1].strip() == "```":
                lines = lines[:-1]
            text = "\n".join(lines).strip()
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            start = text.find("{")
            end = text.rfind("}")
            if start >= 0 and end > start:
                return json.loads(text[start : end + 1])
            raise

    @staticmethod
    def _is_response_format_unsupported(exc: Exception) -> bool:
        message = str(exc).lower()
        status_code = getattr(exc, "status_code", None)
        mentions_format = "response_format" in message or "json_object" in message
        return mentions_format and (status_code in {400, 404, 422} or "unsupported" in message or "unknown" in message)

    @staticmethod
    def _build_prompt(entries: Mapping[str, str], style: str) -> str:
        payload = json.dumps(entries, ensure_ascii=False, indent=2)
        return (
            "Task / 任务：Translate this FTB Quests language map to Simplified Chinese.\n"
            f"Style / 风格：{style}.\n"
            "Return one JSON object with exactly the same keys and translated string values.\n"
            "返回一个 JSON 对象，key 必须与输入完全一致，value 是翻译后的字符串。\n\n"
            "Hard rules / 硬性规则：\n"
            "1. Translate English player-facing text into Simplified Chinese. 将玩家可见英文翻译为简体中文。\n"
            "2. Do not translate JSON keys. 不要翻译 JSON key。\n"
            "3. Opaque placeholders like ⟨P_0⟩, ⟨P_1⟩ are formatting/resource tokens, not words. 占位符是格式或资源标记，不是单词。\n"
            "4. Every placeholder from the input value must appear in the output value exactly once. 输入 value 中每个占位符在输出 value 中必须出现且只出现一次。\n"
            "5. Keep each placeholder byte-for-byte unchanged. Do not remove, rename, duplicate, merge, or reorder characters inside it. 占位符本身必须逐字完全不变。\n"
            "6. Placeholders may wrap a word, e.g. ⟨P_0⟩Nether⟨P_1⟩. Translate the word but keep both wrappers: ⟨P_0⟩下界⟨P_1⟩. 占位符包住的英文也必须翻译，不能因为被包住就保留英文。\n"
            "7. If Chinese word order moves a highlighted phrase, move the whole placeholder-wrapped phrase together. 中文语序变化时，移动整段被占位符包住的短语。\n"
            "8. Preserve item IDs, tags, markdown links, line breaks, escape sequences, numbers, and units. 保留物品 ID、标签、链接、换行、转义、数字和单位。\n\n"
            "Examples / 示例：\n"
            "Input text: Defeat ⟨P_0⟩Ignis⟨P_1⟩ in the ⟨P_2⟩Burning Arena⟨P_3⟩.\n"
            "Good: 在⟨P_2⟩燃烧竞技场⟨P_3⟩中击败⟨P_0⟩伊格尼斯⟨P_1⟩。\n"
            "Bad: 在燃烧竞技场中击败⟨P_0⟩伊格尼斯⟨P_1⟩。  (lost ⟨P_2⟩ and ⟨P_3⟩)\n"
            "Bad: 在⟨P_2⟩燃烧竞技场中击败⟨P_0⟩伊格尼斯⟨P_1⟩。  (lost closing wrapper ⟨P_3⟩)\n"
            "Input text: Found in ⟨P_0⟩Nether⟨P_1⟩.\n"
            "Good: 可在⟨P_0⟩下界⟨P_1⟩找到。\n"
            "Bad: 可在下界找到。  (lost wrappers ⟨P_0⟩ and ⟨P_1⟩)\n\n"
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


# Backward-compatible name for callers that imported the original DeepSeek-only client.
DeepSeekTranslator = OpenAICompatibleTranslator
