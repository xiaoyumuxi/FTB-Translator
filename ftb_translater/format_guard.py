from __future__ import annotations

import json
import re
from collections.abc import Sequence

from ftb_translater.logger import get_logger

_log = get_logger(__name__)

# ---- All protected token patterns (MUST be exactly preserved) ----

# Minecraft colour / formatting codes (& and § variants, including rainbow &z)
_COLOR_PATTERN = re.compile(r"[&§][0-9a-fk-orz]", re.IGNORECASE)

# printf-style format specifiers  e.g. %s, %d, %1$s, %.2f
# The trailing (?!\w) avoids false matches on natural text like "50% faster".
_FORMAT_PATTERN = re.compile(
    r"%(?:\d+\$)?[+#\- 0,(]*\d*(?:\.\d+)?[bcdeEufFgGosxX](?!\w)",
)

# Angle-bracket IDs  e.g. <item:minecraft:stone>, <tag:c:ingots/iron>
_ANGLE_PATTERN = re.compile(r"<[^<>\n]+>")

# Escape sequences  \n \r \t \" \' \\
_ESCAPE_PATTERN = re.compile(r"\\[nrt\"'\\]")

# URLs
_URL_PATTERN = re.compile(r"https?://[^\s\"')\]]+")

# Hex colour codes  #RRGGBB  (not SNBT comments — those start at line begin)
_HEX_PATTERN = re.compile(r"#[0-9a-fA-F]{6}\b")

# All token patterns in priority order (JSON handled separately first)
_FLAT_PATTERNS: list[re.Pattern[str]] = [
    _COLOR_PATTERN,
    _FORMAT_PATTERN,
    _ANGLE_PATTERN,
    _ESCAPE_PATTERN,
    _URL_PATTERN,
    _HEX_PATTERN,
]

# JSON-like object / array pattern (used for detection only)
_JSON_OBJECT_PATTERN = re.compile(r"^\s*[\[{].*[\]}]\s*$", re.DOTALL)


Protection = tuple[str, str]  # (placeholder, original_token)


def protect_text(text: str) -> tuple[str, list[Protection]]:
    """Replace all protected format tokens with numbered placeholders.

    For JSON text components ({...} or [...]), uses structural protection:
    only the human-readable text inside ``"text"`` fields is left exposed
    for translation; everything else is replaced with placeholders.

    Returns (protected_text, [(placeholder, original_token), ...]).
    """
    stripped = text.strip()
    if _looks_like_json(stripped):
        return _protect_json_text(text)
    return _protect_flat_text(text)


def restore_text(text: str, protections: list[Protection]) -> str:
    """Replace ``{P_N}`` placeholders with their original tokens."""
    result = text
    for placeholder, original in protections:
        result = result.replace(placeholder, original)
    return result


def preserved_token_warnings(source: str, translated: str) -> list[str]:
    """Verify that all protected tokens survived translation unchanged.

    Runs a full protect → restore round-trip on the source, then makes
    sure every original token appears in the same order in the translated
    output.  This catches cases where the model ignored placeholders.
    """
    warnings: list[str] = []

    # 1. Control characters — strict count
    for char, name in (("\n", "newline"), ("\r", "carriage return"), ("\t", "tab")):
        if source.count(char) != translated.count(char):
            warnings.append(f"Control character count mismatch for {name}")

    # 2. Token round-trip check
    src_protected, src_protections = protect_text(source)
    tgt_restored = restore_text(translated, src_protections)

    # Every source token must appear in the translated output in the same
    # relative order.  We check by re-protecting the restored output and
    # comparing placeholder-for-placeholder.
    tgt_reprotect, tgt_protections = protect_text(tgt_restored)

    # Compare placeholders pairwise
    if len(src_protections) != len(tgt_protections):
        warnings.append(
            f"Token count mismatch: {len(src_protections)} source vs "
            f"{len(tgt_protections)} translated",
        )
    else:
        for (sp, so), (tp, to) in zip(src_protections, tgt_protections):
            if so != to:
                warnings.append(
                    f"Token mismatch: source={so!r} translated={to!r}",
                )

    # 3. Also check that source tokens still exist in translated
    #    (catches placeholders the model may have dropped entirely)
    for _, original in src_protections:
        if original not in translated:
            warnings.append(f"Missing token in translation: {original!r}")

    if warnings:
        _log.warning(
            "Format token issues in translation (%d): source=%.50r -> target=%.50r",
            len(warnings), source, translated,
        )
    return warnings


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------

_PLACEHOLDER_COUNTER = 0


def _next_placeholder() -> str:
    """Return a distinctive placeholder the model won't translate.

    Uses Unicode angle brackets (U+27E8 / U+27E9) around ``P_N`` so the
    model clearly sees an opaque token to preserve verbatim.
    """
    global _PLACEHOLDER_COUNTER
    ph = f"\u27e8P_{_PLACEHOLDER_COUNTER}\u27e9"
    _PLACEHOLDER_COUNTER += 1
    return ph


def _reset_counter() -> None:
    global _PLACEHOLDER_COUNTER
    _PLACEHOLDER_COUNTER = 0


def _looks_like_json(text: str) -> bool:
    """Quick heuristic: does the string look like a JSON object or array?"""
    return bool(_JSON_OBJECT_PATTERN.match(text))


def _protect_json_text(text: str) -> tuple[str, list[Protection]]:
    """Structural protection for JSON text components.

    Parses the JSON and walks every node.  Only ``"text"`` field values and
    bare string array elements are processed for MC token protection; all
    other keys (color, clickEvent, hoverEvent, …) and their values are left
    entirely untouched.  This guarantees the JSON structure is never altered.
    """
    try:
        obj = json.loads(text)
    except json.JSONDecodeError:
        return _protect_flat_text(text)

    protections: list[Protection] = []

    def _walk(node: object) -> object:
        if isinstance(node, dict):
            new: dict[str, object] = {}
            for key, value in node.items():
                if key == "text" and isinstance(value, str):
                    ptext, psub = _protect_flat_text(value)
                    protections.extend(psub)
                    new[key] = ptext
                elif isinstance(value, (dict, list)):
                    new[key] = _walk(value)
                else:
                    new[key] = value
            return new
        if isinstance(node, list):
            return [_walk(item) for item in node]
        if isinstance(node, str):
            ptext, psub = _protect_flat_text(node)
            protections.extend(psub)
            return ptext
        return node

    obj = _walk(obj)
    result = json.dumps(obj, ensure_ascii=False)
    return result, protections


def _protect_flat_text(text: str) -> tuple[str, list[Protection]]:
    """Flat regex-based protection for plain text strings.

    Every match of every known token pattern is replaced with a
    ``\\x00P_N\\x00`` placeholder.
    """
    protections: list[Protection] = []
    result = text
    for pattern in _FLAT_PATTERNS:
        # Work backwards through matches to preserve positions
        for match in reversed(list(pattern.finditer(result))):
            original = match.group()
            ph = _next_placeholder()
            protections.insert(0, (ph, original))
            result = result[:match.start()] + ph + result[match.end():]
    return result, protections


def _contains_cjk(text: str) -> bool:
    for ch in text:
        if '\u4e00' <= ch <= '\u9fff' or '\u3400' <= ch <= '\u4dbf':
            return True
    return False


def _normalize(tokens: Sequence[str]) -> list[str]:
    return sorted(tokens)
