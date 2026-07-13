from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

from ftb_translater.logger import get_logger

_log = get_logger(__name__)

INLINE_TRANSLATABLE_KEYS = frozenset({"title", "subtitle", "description", "text", "name"})
TRANSLATION_TABLE_KEYS = frozenset({"title", "quest_subtitle", "quest_desc", "chapter_subtitle"})
REFERENCE_VALUE_PATTERN = re.compile(r"^[a-z0-9_.+-]+(?::[a-z0-9_./+-]+)?$")


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
    segments = _ChapterSnbtWalker(text, path).extract()
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


class _ChapterSnbtWalker:
    def __init__(self, text: str, path: Path):
        self.text = text
        self.path = path
        self.index = 0
        self.segments: list[ChapterTextSegment] = []

    def extract(self) -> list[ChapterTextSegment]:
        self._skip_ws_and_comments()
        if self._peek() == "{":
            self._parse_compound()
        else:
            self._parse_entries_until("")
        return self.segments

    def _parse_entries_until(self, end_char: str) -> None:
        while self.index < len(self.text):
            self._skip_ws_and_comments()
            if end_char and self._peek() == end_char:
                self.index += 1
                return
            before = self.index
            if not self._parse_pair():
                self.index = before
                self._skip_unknown_value()
            self._consume_optional_separator()
            if self.index == before:
                self.index += 1

    def _parse_compound(self) -> None:
        if self._peek() != "{":
            self._skip_unknown_value()
            return
        self.index += 1
        self._parse_entries_until("}")

    def _parse_pair(self) -> bool:
        key = self._parse_key()
        if key is None:
            return False
        self._skip_ws_and_comments()
        if self._peek() != ":":
            return False
        self.index += 1
        active_key = _translatable_key(key)
        self._parse_value(active_key)
        return True

    def _parse_key(self) -> str | None:
        self._skip_ws_and_comments()
        if self._peek() in {'"', "'"}:
            value, end, _quote = _parse_string_literal(self.text, self.index)
            self.index = end
            return value
        if not _is_key_start(self._peek()):
            return None
        start = self.index
        while self.index < len(self.text) and _is_key_part(self.text[self.index]):
            self.index += 1
        return self.text[start:self.index]

    def _parse_value(self, active_key: str | None) -> None:
        self._skip_ws_and_comments()
        char = self._peek()
        if not char:
            return
        if char in {'"', "'"}:
            self._parse_string_value(active_key)
            return
        if char == "{":
            self._parse_compound()
            return
        if char == "[":
            self._parse_list(active_key)
            return
        self._skip_atom()

    def _parse_string_value(self, active_key: str | None) -> None:
        start = self.index
        value, end, quote = _parse_string_literal(self.text, start)
        self.index = end
        if active_key is None or not _should_translate(value):
            return
        self.segments.append(
            ChapterTextSegment(
                self.path,
                active_key,
                value,
                start,
                end,
                quote,
                len(self.segments),
            )
        )

    def _parse_list(self, active_key: str | None) -> None:
        if self._peek() != "[":
            return
        self.index += 1
        if self._consume_typed_array_prefix():
            self._skip_until_matching_list_end()
            return
        while self.index < len(self.text):
            self._skip_ws_and_comments()
            if self._peek() == "]":
                self.index += 1
                return
            before = self.index
            self._parse_value(active_key)
            self._consume_optional_separator()
            if self.index == before:
                self.index += 1

    def _consume_typed_array_prefix(self) -> bool:
        checkpoint = self.index
        self._skip_ws_and_comments()
        if self._peek() not in {"B", "I", "L"}:
            self.index = checkpoint
            return False
        array_type_end = self.index + 1
        self.index = array_type_end
        self._skip_ws_and_comments()
        if self._peek() != ";":
            self.index = checkpoint
            return False
        self.index += 1
        return True

    def _skip_until_matching_list_end(self) -> None:
        depth = 1
        while self.index < len(self.text) and depth > 0:
            self._skip_ws_and_comments()
            char = self._peek()
            if not char:
                return
            if char in {'"', "'"}:
                _, end, _ = _parse_string_literal(self.text, self.index)
                self.index = end
                continue
            if char == "[":
                depth += 1
            elif char == "]":
                depth -= 1
            self.index += 1

    def _skip_unknown_value(self) -> None:
        self._skip_ws_and_comments()
        char = self._peek()
        if not char:
            return
        if char in {'"', "'"}:
            _, end, _ = _parse_string_literal(self.text, self.index)
            self.index = end
            return
        if char == "{":
            self._parse_compound()
            return
        if char == "[":
            self._parse_list(None)
            return
        self._skip_atom()

    def _skip_atom(self) -> None:
        while self.index < len(self.text):
            char = self.text[self.index]
            if char.isspace() or char in ",]}{":
                return
            if self.text.startswith("//", self.index):
                return
            if char == "#" and _is_hash_comment(self.text, self.index):
                return
            self.index += 1

    def _consume_optional_separator(self) -> None:
        self._skip_ws_and_comments()
        if self._peek() in {",", ";"}:
            self.index += 1

    def _skip_ws_and_comments(self) -> None:
        while self.index < len(self.text):
            char = self.text[self.index]
            if char.isspace():
                self.index += 1
                continue
            if self.text.startswith("//", self.index):
                self.index = _skip_line_comment(self.text, self.index)
                continue
            if char == "#" and _is_hash_comment(self.text, self.index):
                self.index = _skip_line_comment(self.text, self.index)
                continue
            return

    def _peek(self) -> str:
        if self.index >= len(self.text):
            return ""
        return self.text[self.index]


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


def _is_hash_comment(text: str, position: int) -> bool:
    line_start = text.rfind("\n", 0, position) + 1
    return text[line_start:position].strip() == ""


def _is_key_start(char: str) -> bool:
    return char.isalpha() or char == "_"


def _is_key_part(char: str) -> bool:
    return char.isalnum() or char in "_-.+"


def _translatable_key(key: str) -> str | None:
    if key in INLINE_TRANSLATABLE_KEYS or key in TRANSLATION_TABLE_KEYS:
        return key
    suffix = key.rsplit(".", 1)[-1]
    if suffix in TRANSLATION_TABLE_KEYS:
        return key
    return None


def _should_translate(value: str) -> bool:
    stripped = value.strip()
    if not stripped:
        return False
    if REFERENCE_VALUE_PATTERN.fullmatch(stripped):
        return False
    return any("A" <= char <= "Z" or "a" <= char <= "z" for char in stripped)
