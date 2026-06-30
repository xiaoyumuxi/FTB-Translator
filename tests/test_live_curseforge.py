from __future__ import annotations

import os
import tempfile
import unittest
import urllib.error
import urllib.request
import zipfile
from collections.abc import Mapping
from pathlib import Path
from urllib.parse import urlparse

from ftb_translater.paths import detect_source_mode, resolve_quests_dir, target_lang_path
from ftb_translater.snbt import load_lang_snbt
from ftb_translater.translator import translate_quests_auto


LIVE_TEST_ENV = "FTB_TRANSLATER_LIVE_TEST"
CURSEFORGE_URL_ENV = "FTB_TRANSLATER_CURSEFORGE_URL"
MAX_DOWNLOAD_MB_ENV = "FTB_TRANSLATER_LIVE_MAX_MB"
DEFAULT_CURSEFORGE_URL = "https://edge.forgecdn.net/files/8264/824/ftb-stoneblock-4-1.15.3.zip"
DEFAULT_MAX_DOWNLOAD_MB = 250


class FakeTranslator:
    model = "deepseek-v4-flash"

    def translate_batch(self, entries: Mapping[str, str], style: str) -> dict[str, str]:
        return {key: f"汉化:{value}" for key, value in entries.items()}


@unittest.skipUnless(
    os.getenv(LIVE_TEST_ENV) == "1",
    f"set {LIVE_TEST_ENV}=1 to run the live CurseForge download test",
)
class LiveCurseForgeDownloadTests(unittest.TestCase):
    def test_download_extract_and_translate_real_curseforge_modpack(self) -> None:
        url = os.getenv(CURSEFORGE_URL_ENV) or DEFAULT_CURSEFORGE_URL
        max_bytes = _max_download_bytes()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "curseforge-pack.zip"
            extract_dir = root / "extracted"

            _download(url, archive, max_bytes=max_bytes)
            _safe_extract_zip(archive, extract_dir)

            quests_dir = resolve_quests_dir(extract_dir)
            mode = detect_source_mode(quests_dir)
            report = translate_quests_auto(
                quests_dir,
                api_key="unused",
                batch_size=50,
                translator=FakeTranslator(),
            )

            self.assertGreater(report.total_entries, 0)
            self.assertTrue(Path(report.backup_dir).exists())
            self.assertTrue((quests_dir / ".ftb-translater" / "report-latest.json").exists())
            self.assertTrue((quests_dir / ".ftb-translater" / "cache.json").exists())

            if mode == "lang":
                translated = load_lang_snbt(target_lang_path(quests_dir))
                self.assertTrue(translated)
                self.assertTrue(any(_contains_translation_marker(value) for value in translated.values()))
            else:
                changed_files = [
                    path
                    for path in (quests_dir / "chapters").glob("*.snbt")
                    if "汉化:" in path.read_text(encoding="utf-8")
                ]
                self.assertTrue(changed_files)


def _max_download_bytes() -> int:
    raw_value = os.getenv(MAX_DOWNLOAD_MB_ENV)
    if raw_value is None:
        return DEFAULT_MAX_DOWNLOAD_MB * 1024 * 1024
    try:
        value = int(raw_value)
    except ValueError as exc:
        raise ValueError(f"{MAX_DOWNLOAD_MB_ENV} must be an integer MB value") from exc
    if value <= 0:
        raise ValueError(f"{MAX_DOWNLOAD_MB_ENV} must be greater than zero")
    return value * 1024 * 1024


def _download(url: str, target: Path, max_bytes: int) -> None:
    request = urllib.request.Request(
        url,
        headers={
            "User-Agent": "Mozilla/5.0 FTB-Translater-live-test/1.0",
            "Accept": "application/zip, application/octet-stream, text/html;q=0.8, */*;q=0.5",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=90) as response:  # noqa: S310
            content_length = response.headers.get("Content-Length")
            if content_length and int(content_length) > max_bytes:
                raise AssertionError(
                    f"download is larger than the configured limit: {content_length} bytes"
                )
            total = 0
            with target.open("wb") as output:
                while True:
                    chunk = response.read(1024 * 1024)
                    if not chunk:
                        break
                    total += len(chunk)
                    if total > max_bytes:
                        raise AssertionError(
                            f"download exceeded the configured limit of {max_bytes} bytes"
                        )
                    output.write(chunk)
    except urllib.error.URLError as exc:
        raise AssertionError(f"failed to download CurseForge modpack from {url}: {exc}") from exc

    if not zipfile.is_zipfile(target):
        host = urlparse(url).netloc or url
        sample = target.read_bytes()[:200].decode("utf-8", errors="replace")
        raise AssertionError(
            f"download from {host} did not produce a zip file. "
            f"Set {CURSEFORGE_URL_ENV} to a direct CurseForge/ForgeCDN zip URL. "
            f"Response starts with: {sample!r}"
        )


def _safe_extract_zip(archive: Path, target_dir: Path) -> None:
    target_dir.mkdir(parents=True, exist_ok=True)
    root = target_dir.resolve()
    with zipfile.ZipFile(archive) as zip_file:
        for member in zip_file.infolist():
            destination = (target_dir / member.filename).resolve()
            if root != destination and root not in destination.parents:
                raise AssertionError(f"zip member escapes extraction directory: {member.filename}")
        zip_file.extractall(target_dir)


def _contains_translation_marker(value: str | list[str]) -> bool:
    if isinstance(value, list):
        return any("汉化:" in item for item in value)
    return "汉化:" in value


if __name__ == "__main__":
    unittest.main()
