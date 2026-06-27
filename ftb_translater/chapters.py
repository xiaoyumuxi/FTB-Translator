from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from ftb_translater.logger import get_logger

_log = get_logger(__name__)

TRANSLATABLE_KEYS = frozenset({"title", "subtitle", "description", "text", "name"})


@dataclass(frozen=True)
class ChapterTextSegment:
    path: Path
    key: str
    source_text: str
    start: int
    end: int
    quote: str
    index: int

    @property
    def cache_id(self) -> str:
        return f"{self.path.name}:{self.index}:{self.key}"


def chapter_files(quests_dir: Path) -> list[Path]:
    chapters_dir = quests_dir / "chapters"
    if not chapters_dir.is_dir():
        _log.debug("No chapters directory at %s", chapters_dir)
        return []
    files = sorted(chapters_dir.glob("*.snbt"))
    _log.debug("Found %d chapter files in %s", len(files), chapters_dir)
    return files


def extract_chapter_segments(path: Path) -> list[ChapterTextSegment]:
    _log.debug("Extracting segments from %s", path)
    text = path.read_text(encoding="utf-8-sig")
    segments: list[ChapterTextSegment] = []
    i = 0
    segment_index = 0
    while i < len(text):
        if text.startswith("//", i) or text[i] == "#":
            i = _skip_line_comment(text, i)
            continue
        if text[i] in {'"', "'"}:
            key, end, _ = _parse_string_literal(text, i)
            value_start = _position_after_colon(text, end)
            if value_start is None or key not in TRANSLATABLE_KEYS:
                i = end
                continue
            next_i, found = _extract_value_segments(text, value_start, path, key, segment_index)
            segments.extend(found)
            segment_index += len(found)
            i = max(end, next_i)
            continue
        if _is_key_start(text[i]):
            key_start = i
            while i < len(text) and _is_key_part(text[i]):
                i += 1
            key = text[key_start:i]
            value_start = _position_after_colon(text, i)
            if value_start is None:
                continue
            if key not in TRANSLATABLE_KEYS:
                i = value_start
                continue
            next_i, found = _extract_value_segments(text, value_start, path, key, segment_index)
            segments.extend(found)
            segment_index += len(found)
            i = max(i, next_i)
            continue
        i += 1
    _log.debug("Extracted %d translatable segments from %s", len(segments), path)
    return segments


def replace_chapter_segments(path: Path, translations: dict[int, str]) -> int:
    _log.debug("Replacing %d segments in %s", len(translations), path)
    text = path.read_text(encoding="utf-8-sig")
    segments = extract_chapter_segments(path)
    replacements = [segment for segment in segments if segment.index in translations]
    for segment in sorted(replacements, key=lambda item: item.start, reverse=True):
        literal = _quote_string(translations[segment.index], segment.quote)
        text = text[: segment.start] + literal + text[segment.end :]
    path.write_text(text, encoding="utf-8")
    _log.debug("Replaced %d segments in %s", len(replacements), path)
    return len(replacements)


def count_chapter_segments(quests_dir: Path) -> tuple[int, int]:
    files = chapter_files(quests_dir)
    counts = {path: len(extract_chapter_segments(path)) for path in files}
    total = sum(counts.values())
    _log.debug("Chapter segments count: %d files, %d segments total", len(files), total)
    return len(files), total


def _extract_value_segments(
    text: str,
    value_start: int,
    path: Path,
    key: str,
    segment_index: int,
) -> tuple[int, list[ChapterTextSegment]]:
    i = _skip_ws(text, value_start)
    if i >= len(text):
        return i, []
    if text[i] in {'"', "'"}:
        value, end, quote = _parse_string_literal(text, i)
        if _should_translate(value):
            return end, [ChapterTextSegment(path, key, value, i, end, quote, segment_index)]
        return end, []
    if text[i] == "[":
        return _extract_list_strings(text, i, path, key, segment_index)
    return i, []


def _extract_list_strings(
    text: str,
    start: int,
    path: Path,
    key: str,
    segment_index: int,
) -> tuple[int, list[ChapterTextSegment]]:
    i = start + 1
    depth = 1
    segments: list[ChapterTextSegment] = []
    next_index = segment_index
    while i < len(text) and depth > 0:
        if text.startswith("//", i) or text[i] == "#":
            i = _skip_line_comment(text, i)
            continue
        char = text[i]
        if char in {'"', "'"}:
            value, end, quote = _parse_string_literal(text, i)
            if _should_translate(value):
                segments.append(ChapterTextSegment(path, key, value, i, end, quote, next_index))
                next_index += 1
            i = end
            continue
        if char == "[":
            depth += 1
        elif char == "]":
            depth -= 1
        i += 1
    return i, segments


def _position_after_colon(text: str, position: int) -> int | None:
    i = _skip_ws(text, position)
    if i < len(text) and text[i] == ":":
        return i + 1
    return None


def _parse_string_literal(text: str, start: int) -> tuple[str, int, str]:
    quote = text[start]
    i = start + 1
    chars: list[str] = []
    while i < len(text):
        char = text[i]
        i += 1
        if char == quote:
            return "".join(chars), i, quote
        if char == "\\":
            if i >= len(text):
                chars.append("\\")
                break
            esc = text[i]
            i += 1
            chars.append(_decode_escape(esc))
        else:
            chars.append(char)
    raise ValueError(f"Unterminated string in {start}")


def _quote_string(value: str, quote: str) -> str:
    escaped = (
        value.replace("\\", "\\\\")
        .replace(quote, f"\\{quote}")
        .replace("\n", "\\n")
        .replace("\r", "\\r")
        .replace("\t", "\\t")
    )
    return f"{quote}{escaped}{quote}"


def _decode_escape(esc: str) -> str:
    mapping = {
        "n": "\n",
        "r": "\r",
        "t": "\t",
        "\\": "\\",
        '"': '"',
        "'": "'",
    }
    return mapping.get(esc, esc)


def _skip_line_comment(text: str, start: int) -> int:
    end = text.find("\n", start)
    return len(text) if end == -1 else end + 1


def _skip_ws(text: str, position: int) -> int:
    while position < len(text) and text[position].isspace():
        position += 1
    return position


def _is_key_start(char: str) -> bool:
    return char.isalpha() or char == "_"


def _is_key_part(char: str) -> bool:
    return char.isalnum() or char in "_-.+"


def _should_translate(value: str) -> bool:
    stripped = value.strip()
    if not stripped:
        return False
    return any("A" <= char <= "Z" or "a" <= char <= "z" for char in stripped)
