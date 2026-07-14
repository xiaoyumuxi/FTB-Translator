use crate::{logging, storage::Settings};
use reqwest::{header, Client, Response, StatusCode};
use serde_json::{json, Map, Value};
use std::{collections::HashMap, time::Duration};

pub const OPENAI_COMPATIBLE: &str = "openai_compatible";
pub const DEEPL: &str = "deepl";
pub const GOOGLE_WEB: &str = "google_web";
pub const DEEPL_WEB: &str = "deepl_web";

const GOOGLE_MAX_CHARS: usize = 4500;
const DEEPL_WEB_MAX_CHARS: usize = 1500;
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub fn normalize(provider: &str) -> Result<&str, String> {
    match provider.trim() {
        OPENAI_COMPATIBLE => Ok(OPENAI_COMPATIBLE),
        DEEPL => Ok(DEEPL),
        GOOGLE_WEB => Ok(GOOGLE_WEB),
        DEEPL_WEB => Ok(DEEPL_WEB),
        other => Err(format!("不支持的翻译提供商：{other}")),
    }
}

pub fn requires_api_key(provider: &str) -> bool {
    !matches!(provider, GOOGLE_WEB | DEEPL_WEB)
}

pub fn concurrency_limit(provider: &str) -> Option<usize> {
    matches!(provider, GOOGLE_WEB | DEEPL_WEB).then_some(1)
}

pub async fn request(
    client: &Client,
    settings: &Settings,
    batch: &[(String, String)],
    task_id: &str,
) -> Result<HashMap<String, String>, String> {
    match normalize(&settings.provider)? {
        DEEPL => request_deepl(client, settings, batch, task_id).await,
        GOOGLE_WEB => request_google(client, settings, batch, task_id).await,
        DEEPL_WEB => request_deepl_web(client, settings, batch, task_id).await,
        _ => request_openai(client, settings, batch, task_id).await,
    }
}

async fn request_openai(
    client: &Client,
    s: &Settings,
    batch: &[(String, String)],
    task_id: &str,
) -> Result<HashMap<String, String>, String> {
    let input = batch
        .iter()
        .map(|(id, text)| (id.clone(), Value::String(text.clone())))
        .collect::<Map<_, _>>();
    let prompt = format!(
        "Task / 任务：Translate this FTB Quests language map to Simplified Chinese.\nStyle / 风格：{}。\nReturn one JSON object with exactly the same keys. Opaque placeholders like ⟨P_0⟩ and ⟨G_0⟩ must remain byte-for-byte unchanged and appear exactly once. Preserve item IDs, tags, line breaks, numbers and units.\n\n{}",
        s.style,
        serde_json::to_string_pretty(&input).unwrap()
    );
    let url = format!("{}/chat/completions", s.base_url.trim_end_matches('/'));
    let messages = json!([
        {"role":"system","content":"You are a Minecraft modpack localization assistant. Translate only player-facing English into natural Simplified Chinese. Never modify opaque placeholders; G placeholders are curated Minecraft glossary terms."},
        {"role":"user","content":prompt}
    ]);
    let mut use_response_format = true;
    let mut last = String::new();
    for attempt in 0..3 {
        logging::debug(
            "provider",
            "openai_attempt_started",
            "OpenAI 兼容接口请求尝试开始",
            json!({"task_id":task_id,"provider":s.provider,"attempt":attempt + 1,"batch_size":batch.len()}),
        );
        let mut body = json!({"model":s.model,"messages":messages,"temperature":0.2});
        if use_response_format {
            body["response_format"] = json!({"type":"json_object"});
        }
        match client
            .post(&url)
            .bearer_auth(&s.api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                if !status.is_success() {
                    logging::warn(
                        "provider",
                        "openai_http_failed",
                        "OpenAI 兼容接口返回失败状态",
                        json!({"task_id":task_id,"provider":s.provider,"attempt":attempt + 1,"http_status":status.as_u16()}),
                    );
                    if use_response_format
                        && matches!(
                            status,
                            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY
                        )
                        && (text.contains("response_format") || text.contains("json_object"))
                    {
                        use_response_format = false;
                        continue;
                    }
                    last = format!("HTTP {status}: {text}");
                } else {
                    let value: Value = serde_json::from_str(&text)
                        .map_err(|e| format!("OpenAI 兼容接口返回无效 JSON：{e}"))?;
                    let content = value
                        .pointer("/choices/0/message/content")
                        .and_then(Value::as_str)
                        .ok_or("OpenAI 兼容接口返回内容为空")?;
                    let map = parse_json_map(content)?;
                    if batch.iter().all(|(id, _)| map.contains_key(id)) {
                        logging::debug(
                            "provider",
                            "openai_attempt_completed",
                            "OpenAI 兼容接口请求成功",
                            json!({"task_id":task_id,"provider":s.provider,"attempt":attempt + 1,"returned_entries":map.len()}),
                        );
                        return Ok(map);
                    }
                    last = "OpenAI 兼容接口返回内容缺少条目".into();
                }
            }
            Err(error) => {
                logging::warn(
                    "provider",
                    "openai_network_failed",
                    "OpenAI 兼容接口网络请求失败",
                    json!({"task_id":task_id,"provider":s.provider,"attempt":attempt + 1,"error":error.to_string()}),
                );
                last = error.to_string();
            }
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(800 * (attempt + 1))).await;
        }
    }
    Err(last)
}

fn parse_json_map(content: &str) -> Result<HashMap<String, String>, String> {
    let mut text = content.trim();
    if text.starts_with("```") {
        text = text
            .strip_prefix("```json")
            .or_else(|| text.strip_prefix("```"))
            .unwrap_or(text)
            .trim();
        text = text.strip_suffix("```").unwrap_or(text).trim();
    }
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) {
        text = &text[start..=end];
    }
    serde_json::from_str(text).map_err(|e| format!("翻译接口返回的 JSON 无效：{e}"))
}

async fn request_deepl(
    client: &Client,
    s: &Settings,
    batch: &[(String, String)],
    task_id: &str,
) -> Result<HashMap<String, String>, String> {
    let url = format!("{}/v2/translate", s.base_url.trim_end_matches('/'));
    let body = json!({
        "text": batch.iter().map(|(_, text)| text).collect::<Vec<_>>(),
        "source_lang": "EN",
        "target_lang": "ZH-HANS"
    });
    let response = send_with_retry(DEEPL, task_id, || {
        client
            .post(&url)
            .header(
                header::AUTHORIZATION,
                format!("DeepL-Auth-Key {}", s.api_key),
            )
            .json(&body)
    })
    .await?;
    parse_ordered_translations(response, batch).await
}

async fn request_google(
    client: &Client,
    s: &Settings,
    batch: &[(String, String)],
    task_id: &str,
) -> Result<HashMap<String, String>, String> {
    let mut units = vec![];
    for (id, text) in batch {
        for piece in split_text(text, GOOGLE_MAX_CHARS - 100) {
            units.push((id.clone(), piece));
        }
    }
    let mut result = batch
        .iter()
        .map(|(id, _)| (id.clone(), String::new()))
        .collect::<HashMap<_, _>>();
    let mut chunk = vec![];
    let mut chars = 0;
    for unit in units {
        let size = unit.1.chars().count() + 40;
        if !chunk.is_empty() && chars + size > GOOGLE_MAX_CHARS {
            append_google_chunk(client, s, &chunk, &mut result, task_id).await?;
            chunk.clear();
            chars = 0;
        }
        chunk.push(unit);
        chars += size;
    }
    if !chunk.is_empty() {
        append_google_chunk(client, s, &chunk, &mut result, task_id).await?;
    }
    Ok(result)
}

async fn append_google_chunk(
    client: &Client,
    s: &Settings,
    chunk: &[(String, String)],
    result: &mut HashMap<String, String>,
    task_id: &str,
) -> Result<(), String> {
    let markers = (0..chunk.len())
        .map(|index| format!("⟪FTB_TRANSLATER_BATCH_{index}⟫"))
        .collect::<Vec<_>>();
    let combined = chunk
        .iter()
        .zip(&markers)
        .map(|((_, text), marker)| format!("{marker}{text}"))
        .collect::<Vec<_>>()
        .join("\n");
    let url = format!("{}/translate_a/single", s.base_url.trim_end_matches('/'));
    let form = [
        ("client", "gtx"),
        ("sl", "en"),
        ("tl", "zh-CN"),
        ("dt", "t"),
        ("q", combined.as_str()),
    ];
    let response = send_with_retry(GOOGLE_WEB, task_id, || {
        client
            .post(&url)
            .header(header::USER_AGENT, USER_AGENT)
            .form(&form)
    })
    .await?;
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    let translated = value
        .get(0)
        .and_then(Value::as_array)
        .ok_or("Google 网页翻译返回结构无效")?
        .iter()
        .filter_map(|segment| segment.get(0).and_then(Value::as_str))
        .collect::<String>();
    let parts = split_marked_translation(&translated, &markers)?;
    for ((id, _), translated) in chunk.iter().zip(parts) {
        result.entry(id.clone()).or_default().push_str(&translated);
    }
    Ok(())
}

fn split_marked_translation(text: &str, markers: &[String]) -> Result<Vec<String>, String> {
    let positions = markers
        .iter()
        .map(|marker| text.find(marker))
        .collect::<Option<Vec<_>>>()
        .ok_or("Google 网页翻译未保留批次标记")?;
    if positions.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("Google 网页翻译打乱了批次标记".into());
    }
    let mut result = vec![];
    for (index, marker) in markers.iter().enumerate() {
        let start = positions[index] + marker.len();
        let end = positions.get(index + 1).copied().unwrap_or(text.len());
        let mut value = text[start..end].to_string();
        if index + 1 < markers.len() && value.ends_with('\n') {
            value.pop();
        }
        result.push(value);
    }
    Ok(result)
}

async fn request_deepl_web(
    client: &Client,
    s: &Settings,
    batch: &[(String, String)],
    task_id: &str,
) -> Result<HashMap<String, String>, String> {
    let mut units = vec![];
    for (id, text) in batch {
        for piece in split_text(text, DEEPL_WEB_MAX_CHARS) {
            units.push((id.clone(), piece));
        }
    }
    let mut result = batch
        .iter()
        .map(|(id, _)| (id.clone(), String::new()))
        .collect::<HashMap<_, _>>();
    let mut chunk = vec![];
    let mut chars = 0;
    for unit in units {
        let size = unit.1.chars().count();
        if !chunk.is_empty() && chars + size > DEEPL_WEB_MAX_CHARS {
            append_deepl_web_chunk(client, s, &chunk, &mut result, task_id).await?;
            chunk.clear();
            chars = 0;
        }
        chunk.push(unit);
        chars += size;
    }
    if !chunk.is_empty() {
        append_deepl_web_chunk(client, s, &chunk, &mut result, task_id).await?;
    }
    Ok(result)
}

async fn append_deepl_web_chunk(
    client: &Client,
    s: &Settings,
    chunk: &[(String, String)],
    result: &mut HashMap<String, String>,
    task_id: &str,
) -> Result<(), String> {
    let url = format!("{}/v1/translate", s.base_url.trim_end_matches('/'));
    let body = json!({
        "text": chunk.iter().map(|(_, text)| text).collect::<Vec<_>>(),
        "target_lang": "zh-Hans",
        "source_lang": "en",
        "usage_type": "Translate",
        "app_information": {
            "os": "brex_macOS",
            "os_version": "brex_chrome_120.0.0.0",
            "app_version": "1.86.0",
            "app_build": "chrome_web_store",
            "instance_id": format!("00000000-0000-4000-8000-{:012x}", std::process::id())
        }
    });
    let response = send_with_retry(DEEPL_WEB, task_id, || {
        client
            .post(&url)
            .header(header::AUTHORIZATION, "None")
            .header(
                header::ORIGIN,
                "chrome-extension://cofdbpoegempjloogbagkncekinflcnj",
            )
            .header("Sec-Fetch-Site", "cross-site")
            .header("Sec-Fetch-Mode", "cors")
            .header("Sec-Fetch-Dest", "empty")
            .header(header::USER_AGENT, USER_AGENT)
            .json(&body)
    })
    .await?;
    let values = ordered_translation_values(response).await?;
    if values.len() != chunk.len() {
        return Err("DeepL 网页翻译返回条目数量不一致".into());
    }
    for ((id, _), translated) in chunk.iter().zip(values) {
        result.entry(id.clone()).or_default().push_str(&translated);
    }
    Ok(())
}

async fn parse_ordered_translations(
    response: Response,
    batch: &[(String, String)],
) -> Result<HashMap<String, String>, String> {
    let values = ordered_translation_values(response).await?;
    if values.len() != batch.len() {
        return Err("翻译接口返回条目数量不一致".into());
    }
    Ok(batch
        .iter()
        .zip(values)
        .map(|((id, _), text)| (id.clone(), text))
        .collect())
}

async fn ordered_translation_values(response: Response) -> Result<Vec<String>, String> {
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    value["translations"]
        .as_array()
        .ok_or_else(|| "翻译接口返回结构无效".to_string())?
        .iter()
        .map(|item| {
            item["text"]
                .as_str()
                .map(str::to_string)
                .ok_or("翻译接口返回了无效文本".into())
        })
        .collect()
}

async fn send_with_retry<F>(provider: &str, task_id: &str, mut build: F) -> Result<Response, String>
where
    F: FnMut() -> reqwest::RequestBuilder,
{
    let mut last = String::new();
    for attempt in 0..3 {
        logging::debug(
            "provider",
            "http_attempt_started",
            "翻译接口请求尝试开始",
            json!({"task_id":task_id,"provider":provider,"attempt":attempt + 1}),
        );
        match build().send().await {
            Ok(response) if response.status().is_success() => {
                logging::debug(
                    "provider",
                    "http_attempt_completed",
                    "翻译接口请求成功",
                    json!({"task_id":task_id,"provider":provider,"attempt":attempt + 1,"http_status":response.status().as_u16()}),
                );
                return Ok(response);
            }
            Ok(response) => {
                let status = response.status();
                logging::warn(
                    "provider",
                    "http_attempt_failed",
                    "翻译接口返回失败状态",
                    json!({"task_id":task_id,"provider":provider,"attempt":attempt + 1,"http_status":status.as_u16()}),
                );
                let body = response.text().await.unwrap_or_default();
                last = format!("HTTP {status}: {body}");
            }
            Err(error) => {
                logging::warn(
                    "provider",
                    "http_network_failed",
                    "翻译接口网络请求失败",
                    json!({"task_id":task_id,"provider":provider,"attempt":attempt + 1,"error":error.to_string()}),
                );
                last = error.to_string();
            }
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(1000 * (attempt + 1))).await;
        }
    }
    Err(last)
}

fn split_text(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let chars = text.chars().collect::<Vec<_>>();
    let mut result = vec![];
    let mut start = 0;
    while chars.len() - start > max_chars {
        let end = start + max_chars;
        let mut cut = (start..end)
            .rev()
            .find(|index| {
                matches!(
                    chars[*index],
                    '\n' | '.' | '!' | '?' | '。' | '！' | '？' | ';' | '；' | ' '
                )
            })
            .map(|index| index + 1)
            .filter(|index| *index - start >= max_chars / 2)
            .unwrap_or(end);
        let window = &chars[start..cut];
        let last_open = window.iter().rposition(|c| *c == '⟨');
        let last_close = window.iter().rposition(|c| *c == '⟩');
        if last_open > last_close {
            cut = start + last_open.unwrap();
        }
        if cut <= start {
            cut = end;
        }
        result.push(chars[start..cut].iter().collect());
        start = cut;
    }
    result.push(chars[start..].iter().collect());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_google_markers_back_into_entries() {
        let markers = vec![
            "⟪FTB_TRANSLATER_BATCH_0⟫".into(),
            "⟪FTB_TRANSLATER_BATCH_1⟫".into(),
        ];
        let text = "⟪FTB_TRANSLATER_BATCH_0⟫制作表格\n⟪FTB_TRANSLATER_BATCH_1⟫击败巨龙";
        assert_eq!(
            split_marked_translation(text, &markers).unwrap(),
            vec!["制作表格", "击败巨龙"]
        );
    }

    #[test]
    fn long_text_split_preserves_placeholder() {
        let text = format!("{}⟨P_123⟩{}", "A".repeat(20), "B".repeat(20));
        let chunks = split_text(&text, 25);
        assert_eq!(chunks.concat(), text);
        assert!(chunks
            .iter()
            .all(|chunk| !chunk.contains('⟨') || chunk.contains("⟨P_123⟩")));
    }

    #[test]
    fn parses_fenced_openai_json() {
        let map = parse_json_map("```json\n{\"a\":\"甲\"}\n```").unwrap();
        assert_eq!(map["a"], "甲");
    }

    #[test]
    #[ignore = "calls anonymous web translation services"]
    fn live_web_provider_smoke_test() {
        tauri::async_runtime::block_on(async {
            let client = Client::new();
            let batch = vec![
                ("title".into(), "Craft a table".into()),
                ("desc".into(), "Defeat ⟨P_0⟩Ignis⟨P_1⟩ in the arena".into()),
                ("hint".into(), "Collect ten pieces of stone".into()),
            ];
            for (provider, base_url, model) in [
                (GOOGLE_WEB, "https://translate.googleapis.com", "google-web"),
                (DEEPL_WEB, "https://oneshot-free.www.deepl.com", "deepl-web"),
            ] {
                let settings = Settings {
                    provider: provider.into(),
                    base_url: base_url.into(),
                    model: model.into(),
                    ..Settings::default()
                };
                let translated = request(&client, &settings, &batch, "live-smoke-test")
                    .await
                    .unwrap();
                assert_eq!(translated.len(), batch.len());
                assert!(translated["desc"].contains("⟨P_0⟩"));
                assert!(translated["desc"].contains("⟨P_1⟩"));
            }
        });
    }
}
