from __future__ import annotations

import re

from ftb_translater.logger import get_logger

_log = get_logger(__name__)

TOKEN_PATTERNS = [
    re.compile(r"§[0-9a-fk-or]", re.IGNORECASE),
    re.compile(r"%(?:\d+\$)?[+#\- 0,(]*\d*(?:\.\d+)?[bcdeEufFgGosxX]"),
    re.compile(r"\{[^{}\n]+\}"),
    re.compile(r"<[^<>\n]+>"),
    re.compile(r"#[a-z0-9_.:-]+", re.IGNORECASE),
]


def preserved_token_warnings(source: str, translated: str) -> list[str]:
    warnings: list[str] = []
    for pattern in TOKEN_PATTERNS:
        source_tokens = pattern.findall(source)
        translated_tokens = pattern.findall(translated)
        if _normalize(source_tokens) != _normalize(translated_tokens):
            msg = f"Token mismatch for pattern {pattern.pattern}: source={source_tokens}, translated={translated_tokens}"
            _log.debug(msg)
            warnings.append(f"Token mismatch for pattern {pattern.pattern}")
    if warnings:
        _log.warning(
            "Format token issues in translation (%d): source=%.50r -> target=%.50r",
            len(warnings), source, translated,
        )
    return warnings


def _normalize(tokens: list[str]) -> list[str]:
    return sorted(tokens)
