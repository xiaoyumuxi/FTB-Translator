use crate as crate_root;
use crate::{
    chapters,
    snbt::{self, LangValue},
    storage::{History, Settings},
};
use chrono::Local;
use futures::stream::{self, StreamExt};
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Emitter};
use walkdir::WalkDir;

#[derive(Clone, Debug, Serialize)]
pub struct Scan {
    quests_dir: String,
    pack_name: String,
    mode: String,
    mode_label: String,
    source: String,
    entry_count: usize,
    file_count: usize,
    estimated_batches: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    source_file: String,
    target_file: String,
    backup_dir: String,
    total_entries: usize,
    translated_entries: usize,
    cache_hits: usize,
    failed_entries: Vec<String>,
    warnings: BTreeMap<String, Vec<String>>,
    failed_translations: BTreeMap<String, Value>,
}
#[derive(Clone)]
struct Item {
    id: String,
    source: String,
    protected: String,
    tokens: Vec<(String, String)>,
}

fn has_lang(p: &Path) -> bool {
    p.join("lang/en_us.snbt").is_file()
}
fn has_chapters(p: &Path) -> bool {
    !chapters::files(p).is_empty()
}
pub fn resolve(selected: &Path) -> Result<PathBuf, String> {
    let s = selected
        .canonicalize()
        .map_err(|e| format!("无法打开所选目录：{e}"))?;
    let mut candidates = vec![s.clone()];
    if s.file_name()
        .is_some_and(|x| x == "lang" || x == "chapters")
    {
        if let Some(p) = s.parent() {
            candidates.push(p.into())
        }
    }
    for a in s.ancestors() {
        if a.file_name().is_some_and(|x| x == "quests") {
            candidates.push(a.into())
        }
        if a.file_name().is_some_and(|x| x == "ftbquests") {
            candidates.push(a.join("quests"))
        }
        if a.file_name().is_some_and(|x| x == "config") {
            candidates.push(a.join("ftbquests/quests"))
        }
    }
    candidates.extend([
        s.join("config/ftbquests/quests"),
        s.join("ftbquests/quests"),
        s.join("quests"),
    ]);
    for e in WalkDir::new(&s)
        .max_depth(5)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_dir() && e.file_name() == "quests")
    {
        candidates.push(e.path().into())
    }
    candidates
        .into_iter()
        .find(|p| has_lang(p) || has_chapters(p))
        .ok_or("没有找到 FTB Quests 的 lang/en_us.snbt 或 chapters/*.snbt。".into())
}
fn mode(q: &Path) -> Result<&'static str, String> {
    if has_lang(q) {
        Ok("lang")
    } else if has_chapters(q) {
        Ok("chapters")
    } else {
        Err("任务书目录中没有可翻译内容".into())
    }
}
fn pack_name(q: &Path) -> String {
    q.ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "config"))
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|x| x.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            q.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
}
pub fn scan(payload: &Value) -> Result<Value, String> {
    let q = resolve(Path::new(payload["path"].as_str().unwrap_or("")))?;
    let m = mode(&q)?;
    let (files, count, source) = if m == "lang" {
        let p = q.join("lang/en_us.snbt");
        (1, snbt::load(&p)?.len(), p)
    } else {
        let fs = chapters::files(&q);
        let count = fs
            .iter()
            .map(|p| chapters::extract(p).map(|x| x.len()))
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .sum();
        (fs.len(), count, q.join("chapters"))
    };
    let bs = parse_auto(payload["batch_size"].as_str().unwrap_or("auto"), 25)?;
    Ok(serde_json::to_value(Scan {
        quests_dir: q.display().to_string(),
        pack_name: pack_name(&q),
        mode: m.into(),
        mode_label: if m == "lang" {
            "语言文件"
        } else {
            "章节文件"
        }
        .into(),
        source: source.display().to_string(),
        entry_count: count,
        file_count: files,
        estimated_batches: count.div_ceil(bs),
    })
    .unwrap())
}
fn parse_auto(s: &str, default: usize) -> Result<usize, String> {
    if s.trim().is_empty() || s.eq_ignore_ascii_case("auto") {
        Ok(default)
    } else {
        s.parse::<usize>()
            .ok()
            .filter(|x| *x > 0)
            .ok_or("批大小与并发数必须是 auto 或正整数".into())
    }
}
fn patterns() -> Vec<Regex> {
    [r"(?i)[&§][0-9a-fk-orz]",r"%(?:\d+\$)?[-+# 0,(]*\d*(?:\.\d+)?[bcdeEufFgGosxX]",r"<[^<>\n]+>",r"\{[@A-Za-z][^{}\n]*\}",r"(?i)\b(?:[a-z0-9_.-]+:[a-z0-9_.-]+(?:/[a-z0-9_.-]+)+|(?:assets|config|data|kubejs|models|recipes|textures|ftbquests|chapters|lang|scripts)/[a-z0-9_./-]+|[a-z0-9_.-]+(?:/[a-z0-9_.-]+)+\.[a-z0-9]+)\b",r#"\\[nrt\"'\\]"#,r#"https?://[^\s\"')\]]+"#,r"#[0-9a-fA-F]{6}\b"].iter().map(|x|Regex::new(x).unwrap()).collect()
}
fn protect(text: &str) -> (String, Vec<(String, String)>) {
    let mut found = vec![];
    for re in patterns() {
        for m in re.find_iter(text) {
            found.push((m.start(), m.end(), m.as_str().to_string()))
        }
    }
    found.sort_by_key(|x| x.0);
    found.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    let mut out = String::new();
    let mut last = 0;
    let mut tokens = vec![];
    for (start, end, t) in found {
        if start < last {
            continue;
        }
        out.push_str(&text[last..start]);
        let ph = format!("⟨P_{}⟩", tokens.len());
        out.push_str(&ph);
        tokens.push((ph, t));
        last = end;
    }
    out.push_str(&text[last..]);
    (out, tokens)
}
fn restore(text: &str, tokens: &[(String, String)]) -> String {
    tokens
        .iter()
        .fold(text.to_string(), |s, (p, t)| s.replace(p, t))
}
fn warnings(source: &str, target: &str) -> Vec<String> {
    let (_, st) = protect(source);
    let (_, tt) = protect(target);
    let mut w = vec![];
    for (c, n) in [('\n', "换行"), ('\r', "回车"), ('\t', "制表符")] {
        if source.matches(c).count() != target.matches(c).count() {
            w.push(format!("{n}数量不一致"))
        }
    }
    let mut sc = st.iter().map(|x| x.1.clone()).collect::<Vec<_>>();
    let mut tc = tt.iter().map(|x| x.1.clone()).collect::<Vec<_>>();
    sc.sort();
    tc.sort();
    if sc != tc {
        w.push("格式码、占位符或资源标识不一致".into())
    }
    if let Ok(src) = serde_json::from_str::<Value>(source) {
        match serde_json::from_str::<Value>(target) {
            Ok(tgt) => {
                if json_shape(&src) != json_shape(&tgt) {
                    w.push("JSON 文本组件结构发生变化".into())
                }
            }
            Err(_) => w.push("JSON 文本组件不再是有效 JSON".into()),
        }
    }
    w
}
fn json_shape(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        if k == "text" && v.is_string() {
                            Value::String("$text".into())
                        } else {
                            json_shape(v)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(json_shape).collect()),
        Value::String(s) => Value::String(s.clone()),
        x => x.clone(),
    }
}
fn cache_key(source: &str, s: &Settings) -> String {
    let mut h = Sha256::new();
    h.update(
        json!({"source_text":source,"model":s.model,"target_locale":"zh_cn","style":s.style})
            .to_string(),
    );
    hex::encode(h.finalize())
}
fn load_cache(q: &Path) -> HashMap<String, String> {
    fs::read(q.join(".ftb-translater/cache.json"))
        .ok()
        .and_then(|x| serde_json::from_slice(&x).ok())
        .unwrap_or_default()
}
fn save_cache(q: &Path, c: &HashMap<String, String>) -> Result<(), String> {
    let p = q.join(".ftb-translater/cache.json");
    fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(p, serde_json::to_vec_pretty(c).unwrap()).map_err(|e| e.to_string())
}
fn backup(q: &Path, m: &str) -> Result<PathBuf, String> {
    let root = q
        .join(".ftb-translater/backups")
        .join(Local::now().format("%Y%m%d-%H%M%S").to_string());
    let name = if m == "lang" { "lang" } else { "chapters" };
    for e in WalkDir::new(q.join(name)) {
        let e = e.map_err(|e| e.to_string())?;
        let rel = e.path().strip_prefix(q.join(name)).unwrap();
        let dest = root.join(name).join(rel);
        if e.file_type().is_dir() {
            fs::create_dir_all(&dest).map_err(|e| e.to_string())?
        } else {
            fs::copy(e.path(), dest).map_err(|e| e.to_string())?;
        }
    }
    Ok(root)
}
async fn request(
    client: &Client,
    s: &Settings,
    batch: &[Item],
) -> Result<HashMap<String, String>, String> {
    let input = batch
        .iter()
        .map(|x| (x.id.clone(), Value::String(x.protected.clone())))
        .collect::<Map<_, _>>();
    let prompt=format!("Task / 任务：Translate this FTB Quests language map to Simplified Chinese.\nStyle / 风格：{}。\nReturn one JSON object with exactly the same keys. Opaque placeholders like ⟨P_0⟩ must remain byte-for-byte unchanged and appear exactly once. Preserve item IDs, tags, line breaks, numbers and units.\n\n{}",s.style,serde_json::to_string_pretty(&input).unwrap());
    let url = format!("{}/chat/completions", s.base_url.trim_end_matches('/'));
    let body = json!({"model":s.model,"messages":[{"role":"system","content":"You are a Minecraft modpack localization assistant. Translate only player-facing English into natural Simplified Chinese. Never modify opaque placeholders."},{"role":"user","content":prompt}],"temperature":0.2,"response_format":{"type":"json_object"}});
    let mut last = String::new();
    for attempt in 0..3 {
        match client
            .post(&url)
            .bearer_auth(&s.api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(r) => {
                if !r.status().is_success() {
                    last = format!(
                        "HTTP {}: {}",
                        r.status(),
                        r.text().await.unwrap_or_default()
                    );
                } else {
                    let v: Value = r.json().await.map_err(|e| e.to_string())?;
                    let content = v
                        .pointer("/choices/0/message/content")
                        .and_then(Value::as_str)
                        .ok_or("DeepSeek 返回内容为空")?;
                    let map: HashMap<String, String> = serde_json::from_str(content)
                        .map_err(|e| format!("DeepSeek 返回的 JSON 无效：{e}"))?;
                    if batch.iter().all(|x| map.contains_key(&x.id)) {
                        return Ok(map);
                    }
                    last = "DeepSeek 返回内容缺少条目".into();
                }
            }
            Err(e) => last = e.to_string(),
        }
        tokio::time::sleep(std::time::Duration::from_millis(800 * (attempt + 1))).await;
    }
    Err(last)
}
pub async fn translate(app: AppHandle, data_dir: PathBuf, payload: Value) -> Result<(), String> {
    let q = PathBuf::from(payload["quests_dir"].as_str().ok_or("缺少任务书目录")?);
    let m = mode(&q)?;
    let mut settings = crate_root::storage::load_settings(&data_dir);
    for k in [
        "api_key",
        "base_url",
        "model",
        "style",
        "batch_size",
        "concurrency",
    ] {
        if let Some(v) = payload[k].as_str() {
            match k {
                "api_key" => settings.api_key = v.into(),
                "base_url" => settings.base_url = v.into(),
                "model" => settings.model = v.into(),
                "style" => settings.style = v.into(),
                "batch_size" => settings.batch_size = v.into(),
                _ => settings.concurrency = v.into(),
            }
        }
    }
    if settings.api_key.trim().is_empty() {
        return Err("请先保存 DeepSeek API Key".into());
    }
    let mut items = vec![];
    let mut lang = None;
    let mut chapter_segs = vec![];
    if m == "lang" {
        let map = snbt::load(&q.join("lang/en_us.snbt"))?;
        for (k, v) in &map {
            let source = match v {
                LangValue::Text(x) => x.clone(),
                LangValue::Lines(x) => x.join("\n"),
            };
            let (p, t) = protect(&source);
            items.push(Item {
                id: k.clone(),
                source,
                protected: p,
                tokens: t,
            });
        }
        lang = Some(map)
    } else {
        for file in chapters::files(&q) {
            for s in chapters::extract(&file)? {
                let (p, t) = protect(&s.source);
                items.push(Item {
                    id: s.cache_id.clone(),
                    source: s.source.clone(),
                    protected: p,
                    tokens: t,
                });
                chapter_segs.push(s)
            }
        }
    }
    let mut cache = load_cache(&q);
    let mut results = HashMap::new();
    let mut pending = vec![];
    let mut hits = 0;
    for x in items.clone() {
        if let Some(v) = cache.get(&cache_key(&x.source, &settings)) {
            results.insert(x.id, v.clone());
            hits += 1
        } else {
            pending.push(x)
        }
    }
    let bs = parse_auto(&settings.batch_size, 25)?;
    let concurrency = parse_auto(&settings.concurrency, 6)?.min(12);
    let batches = pending.chunks(bs).map(|x| x.to_vec()).collect::<Vec<_>>();
    let total = pending.len();
    let client = Client::new();
    let app2 = app.clone();
    let settings2 = settings.clone();
    let stream = stream::iter(batches.into_iter().map(|batch| {
        let c = client.clone();
        let s = settings2.clone();
        async move {
            let r = request(&c, &s, &batch).await;
            (batch, r)
        }
    }))
    .buffer_unordered(concurrency);
    tokio::pin!(stream);
    let mut failed = vec![];
    while let Some((batch, r)) = stream.next().await {
        match r {
            Ok(map) => {
                for x in batch {
                    let raw = map.get(&x.id).cloned().unwrap_or(x.protected);
                    let restored = restore(&raw, &x.tokens);
                    results.insert(x.id, restored);
                }
            }
            Err(e) => {
                for x in batch {
                    failed.push(format!("{}: {e}", x.id));
                    results.insert(x.id, x.source);
                }
            }
        }
        let done = results.len().saturating_sub(hits);
        let _ = app2.emit(
            "translation-event",
            json!({"type":"progress","stage":"translating","done":done,"total":total}),
        );
    }
    let backup = backup(&q, m)?;
    let mut warns = BTreeMap::new();
    let mut details = BTreeMap::new();
    for x in &items {
        let translated = results.get(&x.id).unwrap_or(&x.source);
        let w = warnings(&x.source, translated);
        if !w.is_empty() {
            warns.insert(x.id.clone(), w);
            details.insert(x.id.clone(), json!({"source":x.source,"failed":translated}));
            results.insert(x.id.clone(), x.source.clone());
        } else if translated != &x.source {
            cache.insert(cache_key(&x.source, &settings), translated.clone());
        }
    }
    let mut outputs = vec![];
    let (source_file, target_file) = if let Some(mut map) = lang {
        for (k, v) in &mut map {
            if let Some(t) = results.get(k) {
                *v = match v {
                    LangValue::Text(_) => LangValue::Text(t.clone()),
                    LangValue::Lines(_) => {
                        LangValue::Lines(t.split('\n').map(str::to_string).collect())
                    }
                }
            }
        }
        let target = q.join("lang/zh_cn.snbt");
        snbt::write(&target, &map)?;
        let content = fs::read_to_string(&target).map_err(|e| e.to_string())?;
        outputs.push(("lang/zh_cn.snbt".into(), content, json!({})));
        (q.join("lang/en_us.snbt"), target)
    } else {
        let mut by_file: HashMap<PathBuf, Vec<(usize, String)>> = HashMap::new();
        for s in chapter_segs {
            by_file
                .entry(s.path)
                .or_default()
                .push((s.index, results[&s.cache_id].clone()));
        }
        for (file, r) in by_file {
            chapters::replace(&file, &r)?;
            outputs.push((
                format!("chapters/{}", file.file_name().unwrap().to_string_lossy()),
                fs::read_to_string(file).map_err(|e| e.to_string())?,
                json!({}),
            ));
        }
        (q.join("chapters"), q.join("chapters"))
    };
    save_cache(&q, &cache)?;
    let report = Report {
        source_file: source_file.display().to_string(),
        target_file: target_file.display().to_string(),
        backup_dir: backup.display().to_string(),
        total_entries: items.len(),
        translated_entries: items.len() - failed.len(),
        cache_hits: hits,
        failed_entries: failed,
        warnings: warns,
        failed_translations: details,
    };
    let rv = serde_json::to_value(&report).unwrap();
    let id = History::new(&data_dir)?.insert(&q, m, &settings, &rv, &outputs)?;
    fs::create_dir_all(q.join(".ftb-translater")).map_err(|e| e.to_string())?;
    fs::write(
        q.join(".ftb-translater/report-latest.json"),
        serde_json::to_vec_pretty(&rv).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    app.emit(
        "translation-event",
        json!({"type":"done","report":report,"run_id":id}),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn save_review(v: &Value) -> Result<Value, String> {
    let target = PathBuf::from(v["target_file"].as_str().unwrap_or(""));
    let key = v["key"].as_str().unwrap_or("");
    let text = v["text"].as_str().unwrap_or("").trim();
    if text.is_empty() {
        return Err("译文不能为空".into());
    }
    if target.is_file() {
        let mut map = snbt::load(&target)?;
        let entry = map
            .iter_mut()
            .find(|x| x.0 == key)
            .ok_or("目标文件中找不到此条目")?;
        entry.1 = LangValue::Text(text.into());
        snbt::write(&target, &map)?;
    } else {
        let p = key.splitn(3, ':').collect::<Vec<_>>();
        if p.len() != 3 {
            return Err("章节条目标识无效".into());
        }
        chapters::replace(
            &target.join(p[0]),
            &[(p[1].parse().map_err(|_| "章节序号无效")?, text.into())],
        )?;
    }
    Ok(json!({"saved":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    #[test]
    fn protects_format_tokens() {
        let src = "Use &e%s&r on <item:minecraft:stone> in assets/mod/textures/a.png\\n";
        let (p, t) = protect(src);
        assert!(!p.contains("minecraft:stone"));
        assert_eq!(restore(&p, &t), src);
        assert!(warnings(
            src,
            "使用 &e%s&r 于 <item:minecraft:stone>，位于 assets/mod/textures/a.png\\n"
        )
        .is_empty());
    }
    #[test]
    fn rejects_missing_token() {
        assert!(!warnings("Use %s and &eGold&r", "使用黄金").is_empty())
    }
    #[test]
    fn rejects_changed_json_shape() {
        let a = r#"{"text":"Hello","color":"red"}"#;
        let b = r#"{"text":"你好","color":"blue"}"#;
        assert!(!warnings(a, b).is_empty())
    }
    #[test]
    fn scans_lang_pack() {
        let d = tempdir().unwrap();
        let q = d.path().join("config/ftbquests/quests/lang");
        fs::create_dir_all(&q).unwrap();
        fs::write(q.join("en_us.snbt"), "{ title: \"Hello\" }").unwrap();
        let result = scan(&json!({"path":d.path(),"batch_size":"auto"})).unwrap();
        assert_eq!(result["entry_count"], 1);
        assert_eq!(result["mode"], "lang");
    }
}
