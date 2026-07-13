from __future__ import annotations

from collections.abc import Callable

from ftb_translater.deepl_client import DEFAULT_DEEPL_BASE_URL, DEFAULT_DEEPL_MODEL, DeepLTranslator
from ftb_translater.deepseek_client import DEFAULT_BASE_URL, DEFAULT_MODEL, OpenAICompatibleTranslator
from ftb_translater.web_translation_clients import (
    DEEPL_WEB_MODEL,
    GOOGLE_WEB_MODEL,
    DEFAULT_DEEPL_WEB_BASE_URL,
    DEFAULT_GOOGLE_WEB_BASE_URL,
    DeepLWebTranslator,
    GoogleWebTranslator,
)


OPENAI_COMPATIBLE = "openai_compatible"
DEEPL = "deepl"
GOOGLE_WEB = "google_web"
DEEPL_WEB = "deepl_web"
DEFAULT_PROVIDER = OPENAI_COMPATIBLE
PROVIDER_LABELS = {
    OPENAI_COMPATIBLE: "OpenAI 兼容接口",
    DEEPL: "DeepL 翻译 API",
    GOOGLE_WEB: "Google 网页翻译（实验性）",
    DEEPL_WEB: "DeepL 网页翻译（实验性）",
}


def normalize_provider(provider: str | None) -> str:
    value = (provider or DEFAULT_PROVIDER).strip().lower()
    if value not in PROVIDER_LABELS:
        raise ValueError(f"不支持的翻译提供商：{provider}")
    return value


def provider_defaults(provider: str) -> tuple[str, str]:
    normalized = normalize_provider(provider)
    if normalized == DEEPL:
        return DEFAULT_DEEPL_BASE_URL, DEFAULT_DEEPL_MODEL
    if normalized == GOOGLE_WEB:
        return DEFAULT_GOOGLE_WEB_BASE_URL, GOOGLE_WEB_MODEL
    if normalized == DEEPL_WEB:
        return DEFAULT_DEEPL_WEB_BASE_URL, DEEPL_WEB_MODEL
    return DEFAULT_BASE_URL, DEFAULT_MODEL


def provider_cache_id(provider: str, model: str, base_url: str) -> str:
    normalized = normalize_provider(provider)
    if normalized == OPENAI_COMPATIBLE:
        # Keep existing DeepSeek cache entries usable after the migration.
        return model
    return f"{normalized}:{model}:{base_url.rstrip('/')}"


def create_translator(
    provider: str,
    api_key: str,
    model: str,
    base_url: str,
    logger: Callable[[str], None] | None = None,
):
    normalized = normalize_provider(provider)
    if normalized == DEEPL:
        return DeepLTranslator(api_key=api_key, model=model, base_url=base_url, logger=logger)
    if normalized == GOOGLE_WEB:
        return GoogleWebTranslator(model=model, base_url=base_url, logger=logger)
    if normalized == DEEPL_WEB:
        return DeepLWebTranslator(model=model, base_url=base_url, logger=logger)
    return OpenAICompatibleTranslator(api_key=api_key, model=model, base_url=base_url, logger=logger)


def provider_requires_api_key(provider: str) -> bool:
    return normalize_provider(provider) not in {GOOGLE_WEB, DEEPL_WEB}


def provider_max_workers(provider: str) -> int | None:
    if normalize_provider(provider) in {GOOGLE_WEB, DEEPL_WEB}:
        return 1
    return None
