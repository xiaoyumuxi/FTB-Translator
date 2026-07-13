"""API Key 凭证存储。

优先用系统 keyring(macOS Keychain / Windows Credential Manager / Linux Secret Service),
这才是"加密存储"——密钥由系统级安全组件管理。

如果 keyring 后端不可用(例如 headless Linux 没装 Secret Service),会写入一个混淆过的
本地文件作为兜底。这只是为了避免明文落盘,密钥派生自机器特征,本机能读这个文件的攻击
者通常也能推导密钥——因此这不是真正的加密,只是让 API Key 不直接 grep 到。强烈建议
配置可用的 keyring 后端来获得真正的凭证安全。
"""
from __future__ import annotations

import base64
import hashlib
import os
import platform
import uuid
from pathlib import Path

from ftb_translater.logger import get_logger
from ftb_translater.user_paths import user_config_dir

_log = get_logger(__name__)

SERVICE_NAME = "ftb-translater"
ACCOUNT_NAME = "deepseek_api_key"


def _credential_account(provider: str | None = None) -> str:
    if not provider or provider == "openai_compatible":
        return ACCOUNT_NAME
    safe_provider = "".join(char for char in provider.lower() if char.isalnum() or char in {"-", "_"})
    return f"{safe_provider or 'translation'}_api_key"


def _fallback_path(provider: str | None = None) -> Path:
    suffix = "" if not provider or provider == "openai_compatible" else f"-{_credential_account(provider)}"
    return user_config_dir() / f".credential-fallback{suffix}"


def _machine_key() -> bytes:
    """从机器特征派生 Fernet 密钥。同一台机器多次启动稳定,跨机器不同。"""
    seed_parts = [platform.node(), platform.machine(), str(uuid.getnode())]
    seed = "::".join(seed_parts).encode("utf-8")
    digest = hashlib.sha256(seed).digest()
    return base64.urlsafe_b64encode(digest)


def _keyring_available() -> bool:
    try:
        import keyring
        from keyring.errors import NoKeyringError

        backend = keyring.get_keyring()
        if backend is None:
            return False
        if "fail" in type(backend).__name__.lower():
            return False
        return True
    except (ImportError, Exception) as exc:  # noqa: BLE001
        _log.debug("Keyring unavailable: %s", exc)
        return False


def save_api_key(api_key: str, provider: str | None = None) -> str:
    """Save the API key and return the backend actually used."""
    api_key = api_key.strip()
    if _keyring_available():
        try:
            import keyring

            if not api_key:
                try:
                    keyring.delete_password(SERVICE_NAME, _credential_account(provider))
                except Exception:  # noqa: BLE001
                    pass
            else:
                keyring.set_password(SERVICE_NAME, _credential_account(provider), api_key)
            _log.info("API key saved to system keyring")
            _clear_fallback(provider)
            return "keyring"
        except Exception as exc:  # noqa: BLE001
            _log.warning("Keyring save failed, falling back to encrypted file: %s", exc)
    _save_fallback(api_key, provider)
    return "fallback"


def load_api_key(provider: str | None = None) -> str:
    if _keyring_available():
        try:
            import keyring

            value = keyring.get_password(SERVICE_NAME, _credential_account(provider))
            if value:
                return value
        except Exception as exc:  # noqa: BLE001
            _log.warning("Keyring load failed, trying fallback: %s", exc)
    return _load_fallback(provider)


def delete_api_key(provider: str | None = None) -> None:
    if _keyring_available():
        try:
            import keyring

            keyring.delete_password(SERVICE_NAME, _credential_account(provider))
        except Exception:  # noqa: BLE001
            pass
    _clear_fallback(provider)


def has_api_key(provider: str | None = None) -> bool:
    return bool(load_api_key(provider))


def storage_backend_label(backend: str | None = None) -> str:
    """返回当前使用的存储后端友好名,用于 UI 文案。"""
    if backend == "fallback":
        return "本地受限文件(非加密,仅做混淆)"
    if backend == "keyring" or _keyring_available():
        system = platform.system()
        if system == "Darwin":
            return "macOS 钥匙串"
        if system == "Windows":
            return "Windows 凭据管理器"
        if system == "Linux":
            return "系统 Secret Service"
        return "系统凭证管理器"
    return "本地受限文件(非加密,仅做混淆)"


def _save_fallback(api_key: str, provider: str | None = None) -> None:
    from cryptography.fernet import Fernet

    path = _fallback_path(provider)
    path.parent.mkdir(parents=True, exist_ok=True)
    if not api_key:
        _clear_fallback(provider)
        return
    token = Fernet(_machine_key()).encrypt(api_key.encode("utf-8"))
    path.write_bytes(token)
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass
    _log.info("API key saved to encrypted fallback file")


def _load_fallback(provider: str | None = None) -> str:
    from cryptography.fernet import Fernet, InvalidToken

    path = _fallback_path(provider)
    if not path.exists():
        return ""
    try:
        decoded = Fernet(_machine_key()).decrypt(path.read_bytes())
        return decoded.decode("utf-8")
    except (InvalidToken, ValueError) as exc:
        _log.error("Fallback credential corrupt or from another machine: %s", exc)
        return ""


def _clear_fallback(provider: str | None = None) -> None:
    path = _fallback_path(provider)
    if path.exists():
        try:
            path.unlink()
        except OSError as exc:
            _log.warning("Could not delete fallback credential: %s", exc)
