from __future__ import annotations

from collections import OrderedDict
from pathlib import Path
from typing import TypeAlias

from ftb_translater.logger import get_logger

LangValue: TypeAlias = "str | list[str]"
LangMap: TypeAlias = "OrderedDict[str, LangValue]"

_log = get_logger(__name__)


class SnbtParseError(ValueError):
    pass


class _Parser:
    def __init__(self, text: str):
        self.text = text
        self.index = 0

    def parse(self) -> LangMap:
        self._skip_ws_and_comments()
        self._expect("{")
        values: LangMap = OrderedDict()
        while True:
            self._skip_ws_and_comments()
            if self._peek() == "}":
                self.index += 1
                break
            key = self._parse_key()
            self._skip_ws_and_comments()
            self._expect(":")
            self._skip_ws_and_comments()
            value = self._parse_value()
            values[key] = value
            self._skip_ws_and_comments()
            if self._peek() == ",":
                self.index += 1
                continue
            if self._peek() == "}":
                continue
            if self._peek():
                continue
            raise self._error("Expected key or '}'")
        self._skip_ws_and_comments()
        if self.index != len(self.text):
            raise self._error("Unexpected trailing content")
        return values

    def _parse_value(self) -> LangValue:
        if self._peek() in {'"', "'"}:
            return self._parse_string()
        if self._peek() == "[":
            return self._parse_string_list()
        raise self._error("Expected quoted string or string list value")

    def _parse_key(self) -> str:
        if self._peek() in {'"', "'"}:
            return self._parse_string()
        start = self.index
        while self.index < len(self.text) and self.text[self.index] not in ":\r\n\t ":
            self.index += 1
        key = self.text[start:self.index].strip()
        if not key:
            raise self._error("Expected key")
        return key

    def _parse_string(self) -> str:
        quote = self._peek()
        if quote not in {'"', "'"}:
            raise self._error("Expected quoted string value")
        self.index += 1
        chars: list[str] = []
        while self.index < len(self.text):
            char = self.text[self.index]
            self.index += 1
            if char == quote:
                return "".join(chars)
            if char == "\\":
                if self.index >= len(self.text):
                    raise self._error("Unfinished escape sequence")
                esc = self.text[self.index]
                self.index += 1
                chars.append(_decode_escape(esc))
            else:
                chars.append(char)
        raise self._error("Unterminated string")

    def _parse_string_list(self) -> list[str]:
        self._expect("[")
        values: list[str] = []
        while True:
            self._skip_ws_and_comments()
            if self._peek() == "]":
                self.index += 1
                return values
            values.append(self._parse_string())
            self._skip_ws_and_comments()
            if self._peek() == ",":
                self.index += 1
                continue
            if self._peek() == "]":
                continue
            if self._peek():
                continue
            raise self._error("Expected string or ']'")

    def _skip_ws_and_comments(self) -> None:
        while self.index < len(self.text):
            char = self.text[self.index]
            if char.isspace():
                self.index += 1
                continue
            if self.text.startswith("//", self.index):
                self.index = self.text.find("\n", self.index)
                if self.index == -1:
                    self.index = len(self.text)
                continue
            if char == "#":
                self.index = self.text.find("\n", self.index)
                if self.index == -1:
                    self.index = len(self.text)
                continue
            break

    def _expect(self, char: str) -> None:
        if self._peek() != char:
            raise self._error(f"Expected '{char}'")
        self.index += 1

    def _peek(self) -> str:
        if self.index >= len(self.text):
            return ""
        return self.text[self.index]

    def _error(self, message: str) -> SnbtParseError:
        return SnbtParseError(f"{message} at offset {self.index}")


def parse_lang_snbt(text: str) -> LangMap:
    return _Parser(text).parse()


def load_lang_snbt(path: Path) -> LangMap:
    _log.debug("Loading lang SNBT: %s", path)
    try:
        result = parse_lang_snbt(path.read_text(encoding="utf-8-sig"))
        _log.debug("Loaded %d entries from %s", len(result), path)
        return result
    except OSError as exc:
        _log.error("Could not read %s: %s", path, exc)
        raise SnbtParseError(f"Could not read {path}: {exc}") from exc
    except SnbtParseError as exc:
        _log.error("Parse error in %s: %s", path, exc)
        raise


def dump_lang_snbt(values: LangMap | dict[str, LangValue]) -> str:
    lines = ["{"]
    items = list(values.items())
    for index, (key, value) in enumerate(items):
        suffix = "," if index < len(items) - 1 else ""
        if isinstance(value, list):
            lines.append(f'  "{_escape(key)}": [')
            for item_index, item in enumerate(value):
                item_suffix = "," if item_index < len(value) - 1 else ""
                lines.append(f'    "{_escape(item)}"{item_suffix}')
            lines.append(f"  ]{suffix}")
        else:
            lines.append(f'  "{_escape(key)}": "{_escape(value)}"{suffix}')
    lines.append("}")
    return "\n".join(lines) + "\n"


def write_lang_snbt(path: Path, values: LangMap | dict[str, LangValue]) -> None:
    _log.debug("Writing lang SNBT: %s (%d entries)", path, len(values))
    text = dump_lang_snbt(values)
    parsed = parse_lang_snbt(text)
    if list(parsed.keys()) != list(values.keys()):
        _log.error("SNBT round-trip key mismatch when writing %s", path)
        raise SnbtParseError("Written SNBT key set did not validate.")
    path.write_text(text, encoding="utf-8")
    _log.debug("Wrote %s OK", path)


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


def _escape(value: str) -> str:
    return (
        str(value)
        .replace("\\", "\\\\")
        .replace('"', '\\"')
        .replace("\n", "\\n")
        .replace("\r", "\\r")
        .replace("\t", "\\t")
    )
