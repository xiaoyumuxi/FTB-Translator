from __future__ import annotations

from pathlib import Path

from ftb_translater.logger import get_logger

_log = get_logger(__name__)

MAX_SEARCH_DEPTH = 5


def has_lang_source(quests_dir: Path) -> bool:
    return (quests_dir / "lang" / "en_us.snbt").is_file()


def has_chapters_source(quests_dir: Path) -> bool:
    chapters_dir = quests_dir / "chapters"
    return chapters_dir.is_dir() and any(chapters_dir.glob("*.snbt"))


def resolve_quests_dir(selected_dir: Path) -> Path:
    selected_dir = selected_dir.expanduser().resolve()
    _log.debug("resolve_quests_dir: starting from %s", selected_dir)
    candidates = _candidate_quests_dirs(selected_dir)
    _log.debug("Trying %d candidate directories", len(candidates))
    for candidate in candidates:
        if has_lang_source(candidate):
            _log.info("Found lang source at: %s", candidate)
            return candidate
        if has_chapters_source(candidate):
            _log.info("Found chapters source at: %s", candidate)
            return candidate
    _log.error("No FTB quests directory found under %s. Tried: %s", selected_dir, candidates)
    raise FileNotFoundError(
        "Could not find FTB Quests lang/en_us.snbt or chapters/*.snbt. "
        "Select a modpack root, config folder, ftbquests folder, quests folder, lang folder, or chapters folder."
    )


def detect_source_mode(quests_dir: Path) -> str:
    if has_lang_source(quests_dir):
        _log.debug("Source mode: lang (%s)", quests_dir)
        return "lang"
    if has_chapters_source(quests_dir):
        _log.debug("Source mode: chapters (%s)", quests_dir)
        return "chapters"
    _log.error("detect_source_mode: no lang or chapters source found at %s", quests_dir)
    raise FileNotFoundError("Could not find lang/en_us.snbt or chapters/*.snbt.")


def source_lang_path(quests_dir: Path) -> Path:
    return quests_dir / "lang" / "en_us.snbt"


def target_lang_path(quests_dir: Path) -> Path:
    return quests_dir / "lang" / "zh_cn.snbt"


def _candidate_quests_dirs(selected_dir: Path) -> list[Path]:
    candidates: list[Path] = []

    def add(path: Path) -> None:
        resolved = path.resolve()
        if resolved not in candidates:
            candidates.append(resolved)

    add(selected_dir)
    if selected_dir.name.lower() in {"chapters", "lang"}:
        add(selected_dir.parent)

    for parent in [selected_dir, *selected_dir.parents]:
        name = parent.name.lower()
        if name == "quests":
            add(parent)
        if name == "ftbquests":
            add(parent / "quests")
        if name == "config":
            add(parent / "ftbquests" / "quests")

    direct_patterns = [
        selected_dir / "config" / "ftbquests" / "quests",
        selected_dir / "ftbquests" / "quests",
        selected_dir / "quests",
    ]
    for pattern in direct_patterns:
        add(pattern)

    if selected_dir.is_dir():
        for path in selected_dir.rglob("ftbquests"):
            if _relative_depth(selected_dir, path) > MAX_SEARCH_DEPTH:
                continue
            add(path / "quests")
        for path in selected_dir.rglob("quests"):
            if _relative_depth(selected_dir, path) > MAX_SEARCH_DEPTH:
                continue
            add(path)
    return candidates


def _relative_depth(root: Path, child: Path) -> int:
    try:
        return len(child.relative_to(root).parts)
    except ValueError:
        return MAX_SEARCH_DEPTH + 1
