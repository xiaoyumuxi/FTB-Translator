from __future__ import annotations

from pathlib import Path

from ftb_translater.logger import get_logger
from ftb_translater.user_paths import user_config_dir

try:
    from dotenv import dotenv_values
except ImportError:  # pragma: no cover - dependency is declared, fallback keeps imports readable.
    dotenv_values = None


_log = get_logger(__name__)

ENV_KEY = "DEEPSEEK_API_KEY"
BASE_URL_KEY = "DEEPSEEK_BASE_URL"
MODEL_KEY = "DEEPSEEK_MODEL"
STYLE_KEY = "FTB_TRANSLATER_STYLE"
BATCH_SIZE_KEY = "FTB_TRANSLATER_BATCH_SIZE"
CONCURRENCY_KEY = "FTB_TRANSLATER_CONCURRENCY"

APP_CONFIG_KEYS = (
    ENV_KEY,
    BASE_URL_KEY,
    MODEL_KEY,
    STYLE_KEY,
    BATCH_SIZE_KEY,
    CONCURRENCY_KEY,
)


def env_path(base_dir: Path | None = None) -> Path:
    if base_dir is not None:
        return base_dir / ".env"
    return user_config_dir() / ".env"


def load_api_key(base_dir: Path | None = None) -> str:
    path = env_path(base_dir)
    _log.debug("Loading API key from %s", path)
    if dotenv_values is not None and path.exists():
        value = dotenv_values(path).get(ENV_KEY)
        if value:
            _log.debug("API key loaded via python-dotenv")
            return str(value)
    if path.exists():
        value = _read_env_file(path).get(ENV_KEY)
        if value:
            _log.debug("API key loaded from raw app settings file")
            return value
    _log.warning("No API key found in saved app settings")
    return ""


def load_config_values(base_dir: Path | None = None) -> dict[str, str]:
    path = env_path(base_dir)
    values: dict[str, str] = {}
    if dotenv_values is not None and path.exists():
        values.update({key: str(value) for key, value in dotenv_values(path).items() if value is not None})
    elif path.exists():
        values.update(_read_env_file(path))

    for key in APP_CONFIG_KEYS:
        if not values.get(key):
            values[key] = ""
    return values


def save_config_values(values: dict[str, str], base_dir: Path | None = None) -> None:
    path = env_path(base_dir)
    path.parent.mkdir(parents=True, exist_ok=True)
    _log.info("Saving app config to %s", path)

    lines: list[str] = []
    if path.exists():
        lines = path.read_text(encoding="utf-8").splitlines()

    pending = {key: value.strip() for key, value in values.items() if key in APP_CONFIG_KEYS}
    found: set[str] = set()
    next_lines: list[str] = []
    for line in lines:
        stripped = line.strip()
        if "=" not in stripped or stripped.startswith("#"):
            next_lines.append(line)
            continue
        key, _value = stripped.split("=", 1)
        key = key.strip()
        if key in pending:
            next_lines.append(f"{key}={pending[key]}")
            found.add(key)
        else:
            next_lines.append(line)

    for key in APP_CONFIG_KEYS:
        if key in pending and key not in found:
            next_lines.append(f"{key}={pending[key]}")

    path.write_text("\n".join(next_lines).rstrip() + "\n", encoding="utf-8")
    _log.debug("App config saved successfully")


def save_api_key(api_key: str, base_dir: Path | None = None) -> None:
    save_config_values({ENV_KEY: api_key}, base_dir)
    _log.debug("API key saved successfully")


def _read_env_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        value = value.strip().strip('"').strip("'")
        values[key.strip()] = value
    return values
