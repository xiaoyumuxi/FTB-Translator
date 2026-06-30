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

# API Key 现在通过 credential_store(keyring)管理,不再写入 .env
APP_CONFIG_KEYS = (
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
    """已废弃:仅用于从旧 .env 文件迁移读取。新代码应使用 credential_store。"""
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


def save_api_key(api_key: str, base_dir: Path | None = None) -> str:
    """转发到 credential_store。base_dir 参数仅为向后兼容保留,会被忽略。"""
    from ftb_translater import credential_store

    backend = credential_store.save_api_key(api_key)
    _log.debug("API key saved via credential_store")
    return backend


def migrate_api_key_from_env(base_dir: Path | None = None) -> bool:
    """把 .env 里的旧 API Key 迁移到 credential_store,迁移成功后从 .env 删除。

    Returns True 如果完成了迁移。
    """
    from ftb_translater import credential_store

    legacy_key = load_api_key(base_dir)
    if not legacy_key:
        return False

    stored_key = credential_store.load_api_key()
    if stored_key:
        if stored_key == legacy_key:
            _strip_api_key_from_env(base_dir)
            _log.info("Removed duplicate API key from .env after credential migration")
        else:
            _log.warning(
                "Legacy .env API key differs from credential store; keeping .env value for manual review"
            )
        return False

    credential_store.save_api_key(legacy_key)
    _strip_api_key_from_env(base_dir)
    _log.info("Migrated API key from .env to credential store")
    return True


def _strip_api_key_from_env(base_dir: Path | None = None) -> None:
    path = env_path(base_dir)
    if not path.exists():
        return
    lines = path.read_text(encoding="utf-8").splitlines()
    kept = []
    changed = False
    for line in lines:
        stripped = line.strip()
        if "=" in stripped and not stripped.startswith("#"):
            key = stripped.split("=", 1)[0].strip()
            if key == ENV_KEY:
                changed = True
                continue
        kept.append(line)
    if changed:
        path.write_text("\n".join(kept).rstrip() + "\n", encoding="utf-8")


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
