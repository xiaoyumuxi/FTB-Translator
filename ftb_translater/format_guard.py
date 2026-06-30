from __future__ import annotations

import json
import re
import threading
from collections import Counter

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

# FTB/quest text macros and embeds  e.g. {@pagebreak}, {image:ftb:textures/...}
_BRACE_MACRO_PATTERN = re.compile(r"\{[@A-Za-z][^{}\n]*\}")

# Resource identifiers / file paths that can appear outside brace macros.
_RESOURCE_PATH_PATTERN = re.compile(
    r"\b(?:[a-z0-9_.-]+:)?[a-z0-9_.-]+(?:/[a-z0-9_.-]+)+(?:\.[a-z0-9]+)?\b",
    re.IGNORECASE,
)

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
    _BRACE_MACRO_PATTERN,
    _RESOURCE_PATH_PATTERN,
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

    # 2. Token round-trip check.  Colour/style codes are allowed to move as
    # complete segments because Chinese word order often moves highlighted
    # phrases.  Non-colour tokens still require strict relative order.
    _, src_protections = protect_text(source)
    tgt_restored = restore_text(translated, src_protections)
    src_tokens = _extract_protected_tokens(source)
    tgt_tokens = _extract_protected_tokens(tgt_restored)

    src_fixed = [token for token in src_tokens if not _is_movable_style_token(token)]
    tgt_fixed = [token for token in tgt_tokens if not _is_movable_style_token(token)]
    if len(src_fixed) != len(tgt_fixed):
        warnings.append(
            f"Non-colour token count mismatch: {len(src_fixed)} source vs "
            f"{len(tgt_fixed)} translated",
        )
    else:
        for source_token, translated_token in zip(src_fixed, tgt_fixed):
            if source_token != translated_token:
                warnings.append(
                    f"Non-colour token mismatch: source={source_token!r} translated={translated_token!r}",
                )

    src_style = Counter(token for token in src_tokens if _is_movable_style_token(token))
    tgt_style = Counter(token for token in tgt_tokens if _is_movable_style_token(token))
    if src_style != tgt_style:
        warnings.append(f"Colour/style token count mismatch: source={dict(src_style)} translated={dict(tgt_style)}")

    # 3. Strict occurrence check catches dropped duplicate tokens as well as
    # completely missing tokens.
    missing = Counter(src_tokens) - Counter(tgt_tokens)
    for original, count in sorted(missing.items()):
        if count == 1:
            warnings.append(f"Missing token in translation: {original!r}")
        else:
            warnings.append(f"Missing token in translation: {original!r} x{count}")

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
_PLACEHOLDER_LOCK = threading.Lock()


def _next_placeholder() -> str:
    """Return a distinctive placeholder the model won't translate.

    Uses Unicode angle brackets (U+27E8 / U+27E9) around ``P_N`` so the
    model clearly sees an opaque token to preserve verbatim.
    """
    global _PLACEHOLDER_COUNTER
    with _PLACEHOLDER_LOCK:
        ph = f"\u27e8P_{_PLACEHOLDER_COUNTER}\u27e9"
        _PLACEHOLDER_COUNTER += 1
    return ph


def _reset_counter() -> None:
    global _PLACEHOLDER_COUNTER
    with _PLACEHOLDER_LOCK:
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


def _extract_protected_tokens(text: str) -> list[str]:
    """Extract protected tokens in human-readable text order."""
    stripped = text.strip()
    if _looks_like_json(stripped):
        try:
            obj = json.loads(text)
        except json.JSONDecodeError:
            return _extract_flat_tokens(text)

        tokens: list[str] = []

        def _walk(node: object) -> None:
            if isinstance(node, dict):
                for key, value in node.items():
                    if key == "text" and isinstance(value, str):
                        tokens.extend(_extract_flat_tokens(value))
                    elif isinstance(value, (dict, list)):
                        _walk(value)
            elif isinstance(node, list):
                for item in node:
                    if isinstance(item, str):
                        tokens.extend(_extract_flat_tokens(item))
                    else:
                        _walk(item)

        _walk(obj)
        return tokens
    return _extract_flat_tokens(text)


def _extract_flat_tokens(text: str) -> list[str]:
    matches: list[tuple[int, int, int, str]] = []
    for priority, pattern in enumerate(_FLAT_PATTERNS):
        for match in pattern.finditer(text):
            matches.append((match.start(), match.end(), priority, match.group()))

    tokens: list[str] = []
    occupied: list[tuple[int, int]] = []
    for start, end, _priority, token in sorted(matches, key=lambda item: (item[0], item[2], item[1])):
        if any(start < used_end and end > used_start for used_start, used_end in occupied):
            continue
        occupied.append((start, end))
        tokens.append(token)
    return tokens


def _is_movable_style_token(token: str) -> bool:
    return bool(_COLOR_PATTERN.fullmatch(token))
