from __future__ import annotations

import json
import unittest
from unittest.mock import patch
from urllib.parse import parse_qs

from ftb_translater.deepl_client import DeepLTranslator
from ftb_translater.deepseek_client import OpenAICompatibleTranslator
from ftb_translater.providers import DEEPL, OPENAI_COMPATIBLE, create_translator, provider_cache_id
from ftb_translater.providers import DEEPL_WEB, GOOGLE_WEB, provider_max_workers, provider_requires_api_key
from ftb_translater.web_translation_clients import DeepLWebTranslator, GoogleWebTranslator, _split_text


class _Message:
    content = '```json\n{"title": "你好"}\n```'


class _Choice:
    message = _Message()


class _Response:
    choices = [_Choice()]


class _UnsupportedResponseFormat(RuntimeError):
    status_code = 400


class _Completions:
    def __init__(self):
        self.calls: list[dict] = []

    def create(self, **kwargs):
        self.calls.append(kwargs)
        if "response_format" in kwargs:
            raise _UnsupportedResponseFormat("response_format json_object is unsupported")
        return _Response()


class _Client:
    def __init__(self):
        self.chat = type("Chat", (), {"completions": _Completions()})()


class _UrlResponse:
    def __init__(self, payload: dict):
        self.payload = payload

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return None

    def read(self) -> bytes:
        return json.dumps(self.payload, ensure_ascii=False).encode("utf-8")


class ProviderTests(unittest.TestCase):
    def test_openai_compatible_falls_back_without_response_format(self) -> None:
        client = _Client()
        translator = OpenAICompatibleTranslator(api_key="unused", client=client, retries=0)

        result = translator.translate_batch({"title": "Hello"})

        self.assertEqual(result, {"title": "你好"})
        self.assertEqual(len(client.chat.completions.calls), 2)
        self.assertIn("response_format", client.chat.completions.calls[0])
        self.assertNotIn("response_format", client.chat.completions.calls[1])

    def test_deepl_maps_translations_back_to_original_keys(self) -> None:
        response = _UrlResponse({"translations": [{"text": "你好"}, {"text": "世界"}]})
        with patch("ftb_translater.deepl_client.urlopen", return_value=response) as request_mock:
            translator = DeepLTranslator(api_key="key", retries=0)
            result = translator.translate_batch({"a": "Hello", "b": "World"})

        self.assertEqual(result, {"a": "你好", "b": "世界"})
        request = request_mock.call_args.args[0]
        payload = json.loads(request.data.decode("utf-8"))
        self.assertEqual(payload["text"], ["Hello", "World"])
        self.assertEqual(payload["target_lang"], "ZH-HANS")
        self.assertEqual(request.get_header("Authorization"), "DeepL-Auth-Key key")

    def test_provider_factory_and_cache_namespaces(self) -> None:
        openai_client = create_translator(
            OPENAI_COMPATIBLE, "key", "model", "https://example.com/v1"
        )
        deepl_client = create_translator(DEEPL, "key", "deepl", "https://api-free.deepl.com")

        self.assertIsInstance(openai_client, OpenAICompatibleTranslator)
        self.assertIsInstance(deepl_client, DeepLTranslator)
        self.assertEqual(provider_cache_id(OPENAI_COMPATIBLE, "model", "https://a"), "model")
        self.assertNotEqual(
            provider_cache_id(DEEPL, "deepl", "https://api-free.deepl.com"),
            provider_cache_id(DEEPL, "deepl", "https://api.deepl.com"),
        )

    def test_google_web_translator_parses_nested_response_without_key(self) -> None:
        response = _UrlResponse(
            [[
                ['⟪FTB_TRANSLATER_BATCH_0⟫你好\n', 'source', None],
                ['⟪FTB_TRANSLATER_BATCH_1⟫世界', 'source', None],
            ]]
        )
        with patch("ftb_translater.web_translation_clients.urlopen", return_value=response) as request_mock:
            translator = GoogleWebTranslator(retries=0)
            result = translator.translate_batch({"title": "Hello", "desc": "World"})

        self.assertEqual(result, {"title": "你好", "desc": "世界"})
        request = request_mock.call_args.args[0]
        self.assertTrue(request.full_url.endswith("translate_a/single"))
        form = parse_qs(request.data.decode("utf-8"))
        self.assertIn("⟪FTB_TRANSLATER_BATCH_0⟫Hello", form["q"][0])
        self.assertIn("⟪FTB_TRANSLATER_BATCH_1⟫World", form["q"][0])

    def test_deepl_web_translator_uses_anonymous_oneshot_request(self) -> None:
        response = _UrlResponse({"translations": [{"text": "你好"}, {"text": "世界"}]})
        with patch("ftb_translater.web_translation_clients.urlopen", return_value=response) as request_mock:
            translator = DeepLWebTranslator(retries=0)
            result = translator.translate_batch({"a": "Hello", "b": "World"})

        self.assertEqual(result, {"a": "你好", "b": "世界"})
        request = request_mock.call_args.args[0]
        payload = json.loads(request.data.decode("utf-8"))
        self.assertEqual(payload["text"], ["Hello", "World"])
        self.assertEqual(payload["target_lang"], "zh-Hans")
        self.assertEqual(request.get_header("Authorization"), "None")

    def test_web_providers_need_no_key_and_are_rate_limited(self) -> None:
        self.assertFalse(provider_requires_api_key(GOOGLE_WEB))
        self.assertFalse(provider_requires_api_key(DEEPL_WEB))
        self.assertEqual(provider_max_workers(GOOGLE_WEB), 1)
        self.assertEqual(provider_max_workers(DEEPL_WEB), 1)

    def test_deepl_web_split_does_not_cut_placeholders(self) -> None:
        text = "A" * 20 + "⟨P_123⟩" + "B" * 20
        chunks = _split_text(text, 25)

        self.assertEqual("".join(chunks), text)
        self.assertTrue(all("⟨P_123⟩" in chunk or "⟨" not in chunk for chunk in chunks))


if __name__ == "__main__":
    unittest.main()
