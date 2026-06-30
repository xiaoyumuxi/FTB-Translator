from __future__ import annotations

import tempfile
import unittest
from unittest.mock import patch


class CredentialStoreTests(unittest.TestCase):
    def setUp(self) -> None:
        # 隔离到独立配置目录,避免污染用户的真实 keyring/fallback 文件
        self._tmp = tempfile.TemporaryDirectory()
        self._patcher = patch.dict(
            "os.environ", {"FTB_TRANSLATER_CONFIG_DIR": self._tmp.name}
        )
        self._patcher.start()

    def tearDown(self) -> None:
        self._patcher.stop()
        self._tmp.cleanup()

    def test_save_load_delete_via_keyring(self) -> None:
        import keyring
        from keyring.backend import KeyringBackend
        from ftb_translater import credential_store

        class InMemoryKeyring(KeyringBackend):
            priority = 1.0

            def __init__(self) -> None:
                self.store: dict[tuple[str, str], str] = {}

            def get_password(self, service: str, username: str) -> str | None:
                return self.store.get((service, username))

            def set_password(self, service: str, username: str, password: str) -> None:
                self.store[(service, username)] = password

            def delete_password(self, service: str, username: str) -> None:
                self.store.pop((service, username), None)

        original = keyring.get_keyring()
        keyring.set_keyring(InMemoryKeyring())
        try:
            self.assertEqual(credential_store.save_api_key("sk-mem-123"), "keyring")
            self.assertEqual(credential_store.load_api_key(), "sk-mem-123")
            self.assertTrue(credential_store.has_api_key())
            credential_store.delete_api_key()
            self.assertEqual(credential_store.load_api_key(), "")
            self.assertFalse(credential_store.has_api_key())
        finally:
            keyring.set_keyring(original)

    def test_fallback_encrypted_file(self) -> None:
        from ftb_translater import credential_store

        with patch.object(credential_store, "_keyring_available", return_value=False):
            self.assertEqual(credential_store.save_api_key("sk-file-secret"), "fallback")
            self.assertEqual(credential_store.load_api_key(), "sk-file-secret")

            # 文件应存在且不是明文
            path = credential_store._fallback_path()
            self.assertTrue(path.exists())
            content = path.read_bytes()
            self.assertNotIn(b"sk-file-secret", content)

            credential_store.delete_api_key()
            self.assertEqual(credential_store.load_api_key(), "")
            self.assertFalse(path.exists())

    def test_save_empty_clears_credential(self) -> None:
        from ftb_translater import credential_store

        with patch.object(credential_store, "_keyring_available", return_value=False):
            credential_store.save_api_key("sk-tmp")
            credential_store.save_api_key("")
            self.assertEqual(credential_store.load_api_key(), "")


if __name__ == "__main__":
    unittest.main()
