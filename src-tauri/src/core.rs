use crate as crate_root;
use crate::{
    chapters, glossary, providers,
    snbt::{self, LangValue},
    storage::{History, Settings},
};
use chrono::Local;
use futures::stream::{self, StreamExt};
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
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

struct FileOutput {
    path: PathBuf,
    archive_name: String,
    content: String,
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
fn patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS
        .get_or_init(|| {
            [
                r"(?i)[&§][0-9a-fk-orz]",
                r"%(?:\d+\$)?[-+# 0,(]*\d*(?:\.\d+)?[bcdeEufFgGosxX]",
                r"<[^<>\n]+>",
                r"\{[@A-Za-z][^{}\n]*\}",
                r"(?i)\b(?:[a-z0-9_.-]+:[a-z0-9_.-]+(?:/[a-z0-9_.-]+)+|(?:assets|config|data|kubejs|models|recipes|textures|ftbquests|chapters|lang|scripts)/[a-z0-9_./-]+|[a-z0-9_.-]+(?:/[a-z0-9_.-]+)+\.[a-z0-9]+)\b",
                r#"\\[nrt\"'\\]"#,
                r#"https?://[^\s\"')\]]+"#,
                r"#[0-9a-fA-F]{6}\b",
            ]
            .iter()
            .map(|pattern| Regex::new(pattern).expect("format-protection regex must be valid"))
            .collect()
        })
        .as_slice()
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

fn protect_for_translation(
    text: &str,
    glossary: Option<&glossary::Loaded>,
) -> (String, Vec<(String, String)>) {
    let (protected, mut tokens) = protect(text);
    let Some(glossary) = glossary else {
        return (protected, tokens);
    };
    let (protected, glossary_tokens) = glossary.protect(&protected);
    tokens.extend(glossary_tokens);
    (protected, tokens)
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
    let cache_model = if s.provider == providers::OPENAI_COMPATIBLE {
        s.model.clone()
    } else {
        format!(
            "{}:{}:{}",
            s.provider,
            s.model,
            s.base_url.trim_end_matches('/')
        )
    };
    let cache_data = if s.glossary_enabled {
        json!({
            "source_text":source,
            "model":cache_model,
            "target_locale":"zh_cn",
            "style":s.style,
            "glossary_enabled":true,
            "glossary_fingerprint":s.glossary_fingerprint
        })
    } else {
        json!({"source_text":source,"model":cache_model,"target_locale":"zh_cn","style":s.style})
    };
    h.update(cache_data.to_string());
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
fn restore_outputs(snapshots: &[(PathBuf, Option<Vec<u8>>)]) -> Vec<String> {
    snapshots
        .iter()
        .rev()
        .filter_map(|(path, original)| {
            let result = match original {
                Some(content) => fs::write(path, content),
                None => match fs::remove_file(path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error),
                },
            };
            result
                .err()
                .map(|error| format!("{}：{error}", path.display()))
        })
        .collect()
}
fn commit_outputs(outputs: &[FileOutput]) -> Result<(), String> {
    let snapshots = outputs
        .iter()
        .map(|output| match fs::read(&output.path) {
            Ok(content) => Ok((output.path.clone(), Some(content))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok((output.path.clone(), None))
            }
            Err(error) => Err(format!("无法读取 {}：{error}", output.path.display())),
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (index, output) in outputs.iter().enumerate() {
        if let Err(error) = fs::write(&output.path, &output.content) {
            let rollback_errors = restore_outputs(&snapshots[..=index]);
            let rollback = if rollback_errors.is_empty() {
                "已恢复此前写入的文件".to_string()
            } else {
                format!("恢复失败：{}", rollback_errors.join("；"))
            };
            return Err(format!(
                "无法写入 {}：{error}；{rollback}",
                output.path.display()
            ));
        }
    }
    Ok(())
}
async fn request(
    client: &Client,
    s: &Settings,
    batch: &[Item],
) -> Result<HashMap<String, String>, String> {
    let input = batch
        .iter()
        .map(|x| (x.id.clone(), x.protected.clone()))
        .collect::<Vec<_>>();
    providers::request(client, s, &input).await
}
pub async fn translate(app: AppHandle, data_dir: PathBuf, payload: Value) -> Result<(), String> {
    let q = PathBuf::from(payload["quests_dir"].as_str().ok_or("缺少任务书目录")?);
    let m = mode(&q)?;
    let mut settings = crate_root::storage::load_settings(&data_dir);
    for k in [
        "api_key",
        "provider",
        "base_url",
        "model",
        "style",
        "batch_size",
        "concurrency",
        "glossary_path",
    ] {
        if let Some(v) = payload[k].as_str() {
            match k {
                "api_key" => settings.api_key = v.into(),
                "provider" => settings.provider = v.into(),
                "base_url" => settings.base_url = v.into(),
                "model" => settings.model = v.into(),
                "style" => settings.style = v.into(),
                "batch_size" => settings.batch_size = v.into(),
                "glossary_path" => settings.glossary_path = v.into(),
                _ => settings.concurrency = v.into(),
            }
        }
    }
    if let Some(enabled) = payload["glossary_enabled"].as_bool() {
        settings.glossary_enabled = enabled;
    }
    providers::normalize(&settings.provider)?;
    if !providers::requires_api_key(&settings.provider) {
        settings.glossary_enabled = false;
        settings.batch_size = "auto".into();
        settings.concurrency = "auto".into();
    }
    if providers::requires_api_key(&settings.provider) && settings.api_key.trim().is_empty() {
        settings.api_key = crate_root::storage::translation_api_key(&settings.provider)?;
    }
    let loaded_glossary = if settings.glossary_enabled {
        let path = if settings.glossary_path.trim().is_empty() {
            glossary::ensure_default(&data_dir)?
        } else {
            PathBuf::from(&settings.glossary_path)
        };
        let loaded = glossary::Loaded::load(&path)?;
        settings.glossary_path = path.display().to_string();
        settings.glossary_fingerprint = loaded.fingerprint().to_string();
        Some(loaded)
    } else {
        None
    };
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
            let (p, t) = protect_for_translation(&source, loaded_glossary.as_ref());
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
                let (p, t) = protect_for_translation(&s.source, loaded_glossary.as_ref());
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
    let mut concurrency = parse_auto(&settings.concurrency, 6)?.min(12);
    if let Some(limit) = providers::concurrency_limit(&settings.provider) {
        concurrency = concurrency.min(limit);
    }
    let batches = pending.chunks(bs).map(|x| x.to_vec()).collect::<Vec<_>>();
    let total = pending.len();
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| format!("无法初始化翻译网络客户端：{e}"))?;
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
    let mut pending_outputs = vec![];
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
        let content = snbt::dump(&map);
        snbt::parse(&content)?;
        pending_outputs.push(FileOutput {
            path: target.clone(),
            archive_name: "lang/zh_cn.snbt".into(),
            content,
        });
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
            let (content, _) = chapters::render_replacements(&file, &r)?;
            pending_outputs.push(FileOutput {
                archive_name: format!("chapters/{}", file.file_name().unwrap().to_string_lossy()),
                path: file,
                content,
            });
        }
        (q.join("chapters"), q.join("chapters"))
    };
    commit_outputs(&pending_outputs)?;
    let outputs = pending_outputs
        .into_iter()
        .map(|output| (output.archive_name, output.content, json!({})))
        .collect::<Vec<_>>();
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
    let text = v["text"].as_str().unwrap_or("");
    if text.trim().is_empty() {
        return Err("译文不能为空".into());
    }
    if target.is_file() {
        let mut map = snbt::load(&target)?;
        let entry = map
            .iter_mut()
            .find(|x| x.0 == key)
            .ok_or("目标文件中找不到此条目")?;
        entry.1 = match &entry.1 {
            LangValue::Text(_) => LangValue::Text(text.into()),
            LangValue::Lines(_) => LangValue::Lines(text.split('\n').map(str::to_string).collect()),
        };
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
    fn glossary_is_optional_and_restores_curated_terms() {
        let source = "Use Mekanism with an Enchanting Table";
        let (plain, plain_tokens) = protect_for_translation(source, None);
        assert_eq!(restore(&plain, &plain_tokens), source);
        assert!(!plain.contains("⟨G_"));

        let d = tempdir().unwrap();
        let path = glossary::ensure_default(d.path()).unwrap();
        let loaded = glossary::Loaded::load(&path).unwrap();
        let (protected, tokens) = protect_for_translation(source, Some(&loaded));
        assert_eq!(protected.matches("⟨G_").count(), 2);
        assert_eq!(restore(&protected, &tokens), "Use 通用机械 with an 附魔台");
    }
    #[test]
    fn disabled_glossary_keeps_legacy_cache_key() {
        let settings = Settings {
            provider: providers::OPENAI_COMPATIBLE.into(),
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-chat".into(),
            ..Settings::default()
        };
        let mut hash = Sha256::new();
        hash.update(
            json!({
                "source_text":"Mekanism",
                "model":"deepseek-chat",
                "target_locale":"zh_cn",
                "style":settings.style
            })
            .to_string(),
        );
        assert_eq!(
            cache_key("Mekanism", &settings),
            hex::encode(hash.finalize())
        );

        let mut enabled = settings;
        enabled.glossary_enabled = true;
        enabled.glossary_fingerprint = "custom-content-hash".into();
        assert_ne!(
            cache_key("Mekanism", &enabled),
            cache_key("Mekanism", &Settings::default())
        );
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

    #[test]
    fn review_preserves_language_line_arrays() {
        let d = tempdir().unwrap();
        let target = d.path().join("zh_cn.snbt");
        fs::write(
            &target,
            r#"{ "description": ["First line", "Second line"] }"#,
        )
        .unwrap();

        save_review(&json!({
            "target_file": target,
            "key": "description",
            "text": "第一行\n第二行"
        }))
        .unwrap();

        let map = snbt::load(&target).unwrap();
        assert_eq!(
            map[0].1,
            LangValue::Lines(vec!["第一行".into(), "第二行".into()])
        );
    }

    #[test]
    fn failed_output_write_restores_already_written_files() {
        let d = tempdir().unwrap();
        let first = d.path().join("first.snbt");
        fs::write(&first, "original").unwrap();
        let outputs = vec![
            FileOutput {
                path: first.clone(),
                archive_name: "first.snbt".into(),
                content: "translated".into(),
            },
            FileOutput {
                path: d.path().join("missing/second.snbt"),
                archive_name: "second.snbt".into(),
                content: "translated".into(),
            },
        ];

        assert!(commit_outputs(&outputs).is_err());
        assert_eq!(fs::read_to_string(first).unwrap(), "original");
    }
}
