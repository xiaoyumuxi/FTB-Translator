use crate as crate_root;
use crate::{
    chapters, cmp, glossary, logging, providers, rich_text,
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
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    time::{Duration, Instant},
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
    files: Vec<ScanFile>,
    estimated_batches: usize,
}
#[derive(Clone, Debug, Serialize)]
struct ScanFile {
    path: String,
    entry_count: usize,
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
    entry_id: String,
    path: String,
    source: String,
    protected: String,
    tokens: Vec<(String, String)>,
}

struct Entry {
    id: String,
    source: String,
    kind: EntryKind,
}

enum EntryKind {
    Plain(String),
    Untouched(String),
    RichText {
        document: rich_text::Document,
        units: Vec<(String, String)>,
    },
}

impl Entry {
    fn unit_ids(&self) -> Vec<&str> {
        match &self.kind {
            EntryKind::Plain(id) => vec![id],
            EntryKind::Untouched(_) => vec![],
            EntryKind::RichText { units, .. } => units.iter().map(|(_, id)| id.as_str()).collect(),
        }
    }
}

#[derive(Serialize)]
struct TranslationUnitRecord<'a> {
    entry_id: &'a str,
    path: &'a str,
    source: &'a str,
    target: &'a str,
    status: &'static str,
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
    let (files, count, source, file_details) = if m == "lang" {
        let p = q.join("lang/en_us.snbt");
        let count = snbt::load(&p)?.len();
        (
            1,
            count,
            p,
            vec![ScanFile {
                path: "lang/en_us.snbt".into(),
                entry_count: count,
            }],
        )
    } else {
        let fs = chapters::files(&q);
        let file_details = fs
            .iter()
            .map(|path| {
                chapters::extract(path).map(|entries| ScanFile {
                    path: format!(
                        "chapters/{}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    entry_count: entries.len(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let count = file_details.iter().map(|file| file.entry_count).sum();
        (fs.len(), count, q.join("chapters"), file_details)
    };
    let bs = parse_auto(payload["batch_size"].as_str().unwrap_or("auto"), 25)?;
    let scan = Scan {
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
        files: file_details,
        estimated_batches: count.div_ceil(bs),
    };
    logging::info(
        "scanner",
        "scan_completed",
        "任务书扫描完成",
        json!({
            "quests_dir":scan.quests_dir,
            "mode":scan.mode,
            "entry_count":scan.entry_count,
            "file_count":scan.file_count
        }),
    );
    Ok(serde_json::to_value(scan).unwrap())
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
                r"\{\{[^{}\n]+\}\}",
                r"\{[@A-Za-z][^{}\n]*\}",
                r"@[pares]\b(?:\[[^\]\n]*\])?",
                r"#[a-z][a-z0-9_.-]*:[a-z0-9_./-]+",
                r"[a-z][a-z0-9_.-]*:[a-z0-9_./-]+",
                r"(?i)\b(?:[a-z0-9_.-]+:[a-z0-9_.-]+(?:/[a-z0-9_.-]+)+|(?:assets|config|data|kubejs|models|recipes|textures|ftbquests|chapters|lang|scripts)/[a-z0-9_./-]+|[a-z0-9_.-]+(?:/[a-z0-9_.-]+)+\.[a-z0-9]+)\b",
                r#"\\[nrt\"'\\]"#,
                r#"https?://[^\s\"')\]]+"#,
                r"#[0-9a-fA-F]{6}\b",
                r"\b[vV]?\d+(?:\.\d+)+\b",
                r"\b\d+(?:[.,]\d+)?(?:%|[a-zA-Z]+)?\b",
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
fn restore(text: &str, tokens: &[(String, String)]) -> Result<String, String> {
    static OPAQUE_PLACEHOLDER: OnceLock<Regex> = OnceLock::new();
    let placeholder = OPAQUE_PLACEHOLDER
        .get_or_init(|| Regex::new(r"⟨[PG]_\d+⟩").expect("opaque-placeholder regex must be valid"));
    let mut expected = tokens
        .iter()
        .map(|(token, _)| token.clone())
        .collect::<Vec<_>>();
    let mut actual = placeholder
        .find_iter(text)
        .map(|matched| matched.as_str().to_string())
        .collect::<Vec<_>>();
    expected.sort();
    actual.sort();
    if actual != expected {
        return Err("翻译接口修改、增删或重复了不透明占位符".into());
    }
    Ok(tokens
        .iter()
        .fold(text.to_string(), |value, (token, original)| {
            value.replace(token, original)
        }))
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

fn prepare_entry(
    id: String,
    source: String,
    entry_index: usize,
    glossary: Option<&glossary::Loaded>,
) -> (Entry, Vec<Item>) {
    if let Some(document) = rich_text::Document::parse(&source) {
        let mut items = Vec::new();
        let mut units = Vec::new();
        for (unit_index, unit) in document.units().iter().enumerate() {
            let unit_id = format!("__ftb_unit_{entry_index}_{unit_index}");
            let (protected, tokens) = protect_for_translation(&unit.source, glossary);
            items.push(Item {
                id: unit_id.clone(),
                entry_id: id.clone(),
                path: unit.pointer.clone(),
                source: unit.source.clone(),
                protected,
                tokens,
            });
            units.push((unit.pointer.clone(), unit_id));
        }
        return (
            Entry {
                id,
                source,
                kind: EntryKind::RichText { document, units },
            },
            items,
        );
    }
    if rich_text::looks_like_component(&source) {
        return (
            Entry {
                id,
                source,
                kind: EntryKind::Untouched(
                    "疑似 JSON 富文本无法安全解析或包含重复键，已保留原文".into(),
                ),
            },
            vec![],
        );
    }

    let unit_id = format!("__ftb_unit_{entry_index}_0");
    let (protected, tokens) = protect_for_translation(&source, glossary);
    let item = Item {
        id: unit_id.clone(),
        entry_id: id.clone(),
        path: "$".into(),
        source: source.clone(),
        protected,
        tokens,
    };
    (
        Entry {
            id,
            source,
            kind: EntryKind::Plain(unit_id),
        },
        vec![item],
    )
}

fn render_entry(entry: &Entry, results: &HashMap<String, String>) -> Result<String, String> {
    match &entry.kind {
        EntryKind::Untouched(_) => Ok(entry.source.clone()),
        EntryKind::Plain(unit_id) => results
            .get(unit_id)
            .cloned()
            .ok_or_else(|| format!("缺少翻译单元：{unit_id}")),
        EntryKind::RichText { document, units } => {
            if units.is_empty() {
                return Ok(entry.source.clone());
            }
            let translations = units
                .iter()
                .map(|(pointer, unit_id)| {
                    results
                        .get(unit_id)
                        .cloned()
                        .map(|target| (pointer.clone(), target))
                        .ok_or_else(|| format!("缺少 JSON 富文本翻译单元：{unit_id}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            document.render(&translations)
        }
    }
}

fn save_translation_units(
    quests_dir: &Path,
    items: &[Item],
    results: &HashMap<String, String>,
    failure_statuses: &HashMap<String, &'static str>,
) -> Result<(), String> {
    let directory = quests_dir.join(".ftb-translater");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let mut output = String::new();
    for item in items {
        let target = results.get(&item.id).unwrap_or(&item.source);
        let record = TranslationUnitRecord {
            entry_id: &item.entry_id,
            path: &item.path,
            source: &item.source,
            target,
            status: failure_statuses
                .get(&item.entry_id)
                .copied()
                .unwrap_or("translated"),
        };
        output.push_str(&serde_json::to_string(&record).map_err(|e| e.to_string())?);
        output.push('\n');
    }
    fs::write(directory.join("translation-units-latest.jsonl"), output)
        .map_err(|e| format!("无法保存翻译中间文件：{e}"))
}

fn source_fingerprint(entries: &[Entry]) -> String {
    let mut hash = Sha256::new();
    for entry in entries {
        hash.update(entry.id.as_bytes());
        hash.update([0]);
        hash.update(entry.source.as_bytes());
        hash.update([0xff]);
    }
    hex::encode(hash.finalize())
}

fn validate_cmp_identity(
    document: &cmp::Document,
    quests_dir: &Path,
    mode: &str,
) -> Result<(), String> {
    let current = quests_dir.canonicalize().map_err(|e| e.to_string())?;
    let recorded = PathBuf::from(&document.meta.quests_dir)
        .canonicalize()
        .map_err(|e| format!("CMP 中的任务书目录不可用：{e}"))?;
    if recorded != current {
        return Err("CMP 不属于当前扫描的任务书目录".into());
    }
    if document.meta.mode != mode {
        return Err("CMP 的任务书模式与当前目录不一致".into());
    }
    Ok(())
}

fn validate_cmp_source(
    document: &cmp::Document,
    quests_dir: &Path,
    mode: &str,
    entries: &[Entry],
    items: &[Item],
) -> Result<(), String> {
    validate_cmp_identity(document, quests_dir, mode)?;
    if document.meta.total_entries != entries.len()
        || document.meta.source_fingerprint != source_fingerprint(entries)
    {
        return Err("任务书内容在 CMP 生成后发生了变化，请重新扫描并翻译".into());
    }
    if document.records.len() != items.len() {
        return Err("CMP 翻译条目数量与当前任务书不一致".into());
    }
    let expected = items
        .iter()
        .map(|item| ((item.entry_id.as_str(), item.path.as_str()), item))
        .collect::<HashMap<_, _>>();
    for record in &document.records {
        let item = expected
            .get(&(record.entry_id.as_str(), record.path.as_str()))
            .ok_or_else(|| format!("CMP 包含未知回填位置：{} {}", record.entry_id, record.path))?;
        if record.file != entry_source_file(mode, &record.entry_id) {
            return Err(format!("CMP 文件归属被修改：{}", record.entry_id));
        }
        if record.source != item.source {
            return Err(format!(
                "CMP 英文原文被修改：{} {}。只允许修改箭头右侧中文",
                record.entry_id, record.path
            ));
        }
    }
    Ok(())
}

fn entry_source_file(mode: &str, entry_id: &str) -> String {
    if mode == "lang" {
        "lang/en_us.snbt".into()
    } else {
        format!(
            "chapters/{}",
            entry_id.split_once(':').map_or(entry_id, |(file, _)| file)
        )
    }
}

fn cmp_records(
    mode: &str,
    entries: &[Entry],
    items: &[Item],
    results: &HashMap<String, String>,
    warnings: &BTreeMap<String, Vec<String>>,
    failed_entries: &HashSet<String>,
    failure_statuses: &HashMap<String, &'static str>,
) -> Result<Vec<cmp::Record>, String> {
    let entries = entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    items
        .iter()
        .map(|item| {
            let entry = entries
                .get(item.entry_id.as_str())
                .ok_or_else(|| format!("找不到 CMP 条目 {}", item.entry_id))?;
            let target = results.get(&entry.id).unwrap_or(&entry.source);
            let target = match &entry.kind {
                EntryKind::Plain(_) => target.clone(),
                EntryKind::RichText { .. } => rich_text::Document::parse(target)
                    .and_then(|document| document.text_at(&item.path).map(str::to_string))
                    .ok_or_else(|| format!("无法从 JSON 富文本读取 CMP 回填位置：{}", item.path))?,
                EntryKind::Untouched(_) => item.source.clone(),
            };
            Ok(cmp::Record {
                file: entry_source_file(mode, &item.entry_id),
                entry_id: item.entry_id.clone(),
                path: item.path.clone(),
                source: item.source.clone(),
                status: if let Some(status) = failure_statuses.get(&item.entry_id) {
                    (*status).into()
                } else if warnings.contains_key(&item.entry_id)
                    || failed_entries.contains(&item.entry_id)
                {
                    "review".into()
                } else if target == item.source {
                    "unchanged".into()
                } else {
                    "translated".into()
                },
                target,
            })
        })
        .collect()
}

fn request_failure_status(error: &str) -> &'static str {
    if error.contains("HTTP 429") || error.to_ascii_lowercase().contains("rate limit") {
        "rate_limited"
    } else {
        "request_failed"
    }
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
    if let Some(src) = rich_text::Document::parse(source) {
        match rich_text::Document::parse(target) {
            Some(tgt) => {
                if src.structure() != tgt.structure() {
                    w.push("JSON 文本组件结构发生变化".into())
                }
            }
            None => w.push("JSON 文本组件不再是有效 JSON".into()),
        }
    }
    w
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
        let mut value = json!({
            "source_text":source,
            "model":cache_model,
            "target_locale":"zh_cn",
            "style":s.style,
            "glossary_enabled":true,
            "glossary_fingerprint":s.glossary_fingerprint
        });
        if rich_text::Document::parse(source).is_some_and(|document| !document.units().is_empty()) {
            value["rich_text_pipeline"] = json!("display-fields-v1");
        }
        value
    } else {
        let mut value = json!({"source_text":source,"model":cache_model,"target_locale":"zh_cn","style":s.style});
        if rich_text::Document::parse(source).is_some_and(|document| !document.units().is_empty()) {
            value["rich_text_pipeline"] = json!("display-fields-v1");
        }
        value
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
    let parent = q.join(".ftb-translater/backups");
    fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let timestamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let root = (0..1000)
        .find_map(|attempt| {
            let name = if attempt == 0 {
                timestamp.clone()
            } else {
                format!("{timestamp}-{attempt:03}")
            };
            let candidate = parent.join(name);
            match fs::create_dir(&candidate) {
                Ok(()) => Some(Ok(candidate)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error.to_string())),
            }
        })
        .ok_or("无法创建唯一备份目录")??;
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
fn commit_outputs(outputs: &[FileOutput], task_id: &str) -> Result<(), String> {
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
            logging::error(
                "translation",
                "output_commit_failed",
                "输出文件提交失败并已尝试回滚",
                json!({
                    "task_id":task_id,
                    "path":output.path,
                    "written_before_failure":index,
                    "rollback_succeeded":rollback_errors.is_empty(),
                    "rollback_error_count":rollback_errors.len(),
                    "error":error.to_string()
                }),
            );
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
    task_id: &str,
) -> Result<HashMap<String, String>, String> {
    let input = batch
        .iter()
        .map(|x| (x.id.clone(), x.protected.clone()))
        .collect::<Vec<_>>();
    providers::request(client, s, &input, task_id).await
}
pub async fn translate(app: AppHandle, data_dir: PathBuf, payload: Value) -> Result<(), String> {
    let started_at = Instant::now();
    let task_id = payload["_task_id"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(logging::task_id);
    let q = PathBuf::from(payload["quests_dir"].as_str().ok_or("缺少任务书目录")?);
    let retry_document = payload["retry_cmp_path"]
        .as_str()
        .filter(|path| !path.trim().is_empty())
        .map(|path| cmp::load(Path::new(path)))
        .transpose()?;
    let retry_locations = retry_document
        .as_ref()
        .map_or_else(HashSet::new, |document| {
            document
                .records
                .iter()
                .filter(|record| record.status == "rate_limited")
                .map(|record| (record.entry_id.clone(), record.path.clone()))
                .collect()
        });
    if payload["retry_cmp_path"].is_string() && retry_locations.is_empty() {
        return Err("CMP 中没有可重试的限流条目".into());
    }
    let m = mode(&q)?;
    if let Some(document) = &retry_document {
        validate_cmp_identity(document, &q, m)?;
    }
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
    logging::info(
        "translation",
        "task_started",
        "翻译任务已开始",
        json!({
            "task_id":task_id,
            "quests_dir":q,
            "mode":m,
            "provider":settings.provider,
            "model":settings.model,
            "glossary_enabled":settings.glossary_enabled
        }),
    );
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
    let mut entries = vec![];
    let mut items = vec![];
    if m == "lang" {
        let map = snbt::load(&q.join("lang/en_us.snbt"))?;
        for (entry_index, (k, v)) in map.iter().enumerate() {
            let source = match v {
                LangValue::Text(x) => x.clone(),
                LangValue::Lines(x) => x.join("\n"),
            };
            let (entry, entry_items) =
                prepare_entry(k.clone(), source, entry_index, loaded_glossary.as_ref());
            entries.push(entry);
            items.extend(entry_items);
        }
    } else {
        let mut entry_index = 0;
        for file in chapters::files(&q) {
            for s in chapters::extract(&file)? {
                let (entry, entry_items) = prepare_entry(
                    s.cache_id.clone(),
                    s.source.clone(),
                    entry_index,
                    loaded_glossary.as_ref(),
                );
                entries.push(entry);
                items.extend(entry_items);
                entry_index += 1;
            }
        }
    }
    if let Some(document) = &retry_document {
        validate_cmp_source(document, &q, m, &entries, &items)?;
    }
    logging::info(
        "translation",
        "content_prepared",
        "待翻译内容已解析",
        json!({"task_id":task_id,"entries":entries.len(),"translation_units":items.len()}),
    );
    for item in &items {
        logging::trace(
            "translation",
            "unit_prepared",
            "翻译单元已准备",
            json!({"task_id":task_id,"entry_id":item.entry_id,"path":item.path}),
        );
    }
    let mut cache = load_cache(&q);
    let items_by_id = items
        .iter()
        .cloned()
        .map(|item| (item.id.clone(), item))
        .collect::<HashMap<_, _>>();
    let mut results = HashMap::new();
    let mut pending = vec![];
    let mut hits = 0;
    for entry in &entries {
        if entry.unit_ids().is_empty() {
            results.insert(entry.id.clone(), entry.source.clone());
            continue;
        }
        if let Some(value) = cache.get(&cache_key(&entry.source, &settings)) {
            results.insert(entry.id.clone(), value.clone());
            hits += 1
        } else {
            pending.extend(
                entry
                    .unit_ids()
                    .into_iter()
                    .filter_map(|id| items_by_id.get(id).cloned()),
            );
        }
    }
    if !retry_locations.is_empty() {
        pending
            .retain(|item| retry_locations.contains(&(item.entry_id.clone(), item.path.clone())));
    }
    let bs = parse_auto(&settings.batch_size, 25)?;
    let mut concurrency = parse_auto(&settings.concurrency, 6)?.min(12);
    if let Some(limit) = providers::concurrency_limit(&settings.provider) {
        concurrency = concurrency.min(limit);
    }
    let batches = pending.chunks(bs).map(|x| x.to_vec()).collect::<Vec<_>>();
    let batch_count = batches.len();
    let total = pending.len();
    logging::info(
        "translation",
        "execution_plan",
        "翻译执行计划已生成",
        json!({
            "task_id":task_id,
            "entries":entries.len(),
            "cache_hits":hits,
            "pending_units":total,
            "batch_size":bs,
            "batch_count":batch_count,
            "concurrency":concurrency
        }),
    );
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| format!("无法初始化翻译网络客户端：{e}"))?;
    let app2 = app.clone();
    let settings2 = settings.clone();
    let stream = stream::iter(batches.into_iter().enumerate().map(|(batch_index, batch)| {
        let c = client.clone();
        let s = settings2.clone();
        let task_id = task_id.clone();
        async move {
            let batch_started = Instant::now();
            logging::debug(
                "provider",
                "batch_request_started",
                "翻译批次请求已发送",
                json!({
                    "task_id":task_id,
                    "batch_index":batch_index + 1,
                    "entries":batch.len(),
                    "provider":s.provider
                }),
            );
            let r = request(&c, &s, &batch, &task_id).await;
            match &r {
                Ok(values) => logging::debug(
                    "provider",
                    "batch_request_completed",
                    "翻译批次请求已完成",
                    json!({
                        "task_id":task_id,
                        "batch_index":batch_index + 1,
                        "returned_entries":values.len(),
                        "duration_ms":batch_started.elapsed().as_millis()
                    }),
                ),
                Err(error) => logging::warn(
                    "provider",
                    "batch_request_failed",
                    "翻译批次请求失败",
                    json!({
                        "task_id":task_id,
                        "batch_index":batch_index + 1,
                        "duration_ms":batch_started.elapsed().as_millis(),
                        "error":error
                    }),
                ),
            }
            (batch, r)
        }
    }))
    .buffer_unordered(concurrency);
    tokio::pin!(stream);
    let mut unit_results = HashMap::new();
    let mut failed_units = HashSet::new();
    let mut failed_entries = HashSet::new();
    let mut failure_statuses = HashMap::new();
    while let Some((batch, r)) = stream.next().await {
        match r {
            Ok(map) => {
                for x in batch {
                    let raw = map.get(&x.id).cloned().unwrap_or(x.protected);
                    match restore(&raw, &x.tokens) {
                        Ok(restored) => {
                            let _ = app2.emit(
                                "translation-event",
                                json!({
                                    "type":"translation_preview",
                                    "task_id":task_id,
                                    "entry_id":x.entry_id,
                                    "source":x.source,
                                    "target":restored,
                                    "status":"translated",
                                }),
                            );
                            unit_results.insert(x.id, restored);
                            let _ = app2.emit(
                                "translation-event",
                                json!({"type":"progress","task_id":task_id,"stage":"translating","done":unit_results.len(),"total":total}),
                            );
                        }
                        Err(error) => {
                            logging::warn(
                                "translation",
                                "placeholder_restore_failed",
                                "翻译单元占位符恢复失败",
                                json!({"task_id":task_id,"entry_id":x.entry_id,"path":x.path,"error":error}),
                            );
                            failed_entries.insert(x.entry_id.clone());
                            failure_statuses.insert(x.entry_id.clone(), "format_guard");
                            failed_units.insert(x.id.clone());
                            unit_results.insert(x.id, x.source);
                            let _ = app2.emit(
                                "translation-event",
                                json!({"type":"progress","task_id":task_id,"stage":"format_guard","done":unit_results.len(),"total":total}),
                            );
                        }
                    }
                }
            }
            Err(e) => {
                let status = request_failure_status(&e);
                for x in batch {
                    logging::warn(
                        "translation",
                        "unit_translation_failed",
                        "翻译单元请求失败",
                        json!({"task_id":task_id,"entry_id":x.entry_id,"path":x.path,"error":&e}),
                    );
                    failed_entries.insert(x.entry_id.clone());
                    failure_statuses.insert(x.entry_id.clone(), status);
                    failed_units.insert(x.id.clone());
                    unit_results.insert(x.id, x.source);
                    let _ = app2.emit(
                        "translation-event",
                        json!({"type":"progress","task_id":task_id,"stage":status,"done":unit_results.len(),"total":total}),
                    );
                }
            }
        }
    }
    save_translation_units(&q, &pending, &unit_results, &failure_statuses)?;
    logging::debug(
        "translation",
        "provider_phase_completed",
        "所有翻译批次已处理",
        json!({
            "task_id":task_id,
            "completed_units":unit_results.len(),
            "failed_units":failed_units.len()
        }),
    );
    for entry in &entries {
        if results.contains_key(&entry.id) {
            continue;
        }
        match render_entry(entry, &unit_results) {
            Ok(target) => {
                results.insert(entry.id.clone(), target);
            }
            Err(error) => {
                logging::warn(
                    "translation",
                    "entry_render_failed",
                    "翻译条目重建失败",
                    json!({"task_id":task_id,"entry_id":entry.id,"error":error}),
                );
                failed_entries.insert(entry.id.clone());
                results.insert(entry.id.clone(), entry.source.clone());
            }
        }
    }
    let mut warns = BTreeMap::new();
    for entry in &entries {
        if let EntryKind::Untouched(reason) = &entry.kind {
            warns.insert(entry.id.clone(), vec![reason.clone()]);
            failed_entries.insert(entry.id.clone());
            continue;
        }
        let translated = results.get(&entry.id).unwrap_or(&entry.source);
        let w = warnings(&entry.source, translated);
        if !w.is_empty() {
            warns.insert(entry.id.clone(), w);
            results.insert(entry.id.clone(), entry.source.clone());
        } else if translated != &entry.source {
            cache.insert(cache_key(&entry.source, &settings), translated.clone());
        }
    }
    save_cache(&q, &cache)?;
    let mut records = cmp_records(
        m,
        &entries,
        &items,
        &results,
        &warns,
        &failed_entries,
        &failure_statuses,
    )?;
    if let Some(previous) = &retry_document {
        let generated = records
            .into_iter()
            .map(|record| ((record.entry_id.clone(), record.path.clone()), record))
            .collect::<HashMap<_, _>>();
        records = previous
            .records
            .iter()
            .map(|record| {
                if retry_locations.contains(&(record.entry_id.clone(), record.path.clone())) {
                    generated
                        .get(&(record.entry_id.clone(), record.path.clone()))
                        .cloned()
                        .unwrap_or_else(|| record.clone())
                } else {
                    record.clone()
                }
            })
            .collect();
    }
    let cmp_path = payload["retry_cmp_path"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            q.join(".ftb-translater/reviews").join(format!(
                "translation-{}.cmp",
                Local::now().format("%Y%m%d-%H%M%S")
            ))
        });
    cmp::write(
        &cmp_path,
        &cmp::Document {
            meta: cmp::Meta {
                version: 1,
                task_id: task_id.clone(),
                quests_dir: q.display().to_string(),
                mode: m.into(),
                source_fingerprint: source_fingerprint(&entries),
                provider: settings.provider.clone(),
                base_url: settings.base_url.clone(),
                model: settings.model.clone(),
                style: settings.style.clone(),
                glossary_enabled: settings.glossary_enabled,
                glossary_fingerprint: settings.glossary_fingerprint.clone(),
                total_entries: entries.len(),
                cache_hits: hits,
            },
            records,
        },
    )?;
    app.emit(
        "translation-event",
        json!({
            "type":"review_ready",
            "task_id":task_id,
            "cmp_path":cmp_path,
            "total_entries":entries.len(),
            "warning_count":warns.len(),
            "failed_count":failed_entries.len()
        }),
    )
    .map_err(|e| e.to_string())?;
    logging::info(
        "translation",
        "review_ready",
        "CMP 人工校对文件已生成",
        json!({
            "task_id":task_id,
            "cmp_path":cmp_path,
            "total_entries":entries.len(),
            "cache_hits":hits,
            "failed_entries":failed_entries.len(),
            "warnings":warns.len(),
            "duration_ms":started_at.elapsed().as_millis()
        }),
    );
    Ok(())
}

pub fn apply_cmp(data_dir: &Path, payload: &Value) -> Result<Value, String> {
    let cmp_path = PathBuf::from(payload["cmp_path"].as_str().ok_or("缺少 CMP 文件路径")?);
    let operation_id = logging::task_id();
    let document = match cmp::load(&cmp_path) {
        Ok(document) => document,
        Err(error) => {
            logging::warn(
                "translation",
                "cmp_load_failed",
                "CMP 校对文件读取或解析失败",
                json!({"task_id":operation_id,"cmp_path":cmp_path,"error":error}),
            );
            return Err(error);
        }
    };
    let task_id = if document.meta.task_id.trim().is_empty() {
        operation_id
    } else {
        document.meta.task_id.clone()
    };
    logging::info(
        "translation",
        "cmp_apply_started",
        "CMP 校验与写回流程已开始",
        json!({"task_id":task_id,"cmp_path":cmp_path}),
    );
    logging::info(
        "translation",
        "cmp_validation_started",
        "开始重新扫描源文件并校验 CMP",
        json!({"task_id":task_id,"cmp_path":cmp_path}),
    );
    let phase = std::cell::Cell::new("validation");
    let result = apply_cmp_inner(
        data_dir,
        payload,
        cmp_path.clone(),
        document,
        &task_id,
        &phase,
    );
    if let Err(error) = &result {
        logging::warn(
            "translation",
            "cmp_apply_failed",
            "CMP 校验或写回流程失败",
            json!({"task_id":task_id,"cmp_path":cmp_path,"phase":phase.get(),"error":error}),
        );
    }
    result
}

fn apply_cmp_inner(
    data_dir: &Path,
    payload: &Value,
    cmp_path: PathBuf,
    document: cmp::Document,
    task_id: &str,
    phase: &std::cell::Cell<&str>,
) -> Result<Value, String> {
    let selected = Path::new(payload["quests_dir"].as_str().ok_or("缺少任务书目录")?);
    let q = resolve(selected)?;
    let m = mode(&q)?;
    validate_cmp_identity(&document, &q, m)?;

    let mut entries = Vec::new();
    let mut items = Vec::new();
    let mut lang = None;
    let mut chapter_segs = Vec::new();
    if m == "lang" {
        let map = snbt::load(&q.join("lang/en_us.snbt"))?;
        for (entry_index, (key, value)) in map.iter().enumerate() {
            let source = match value {
                LangValue::Text(value) => value.clone(),
                LangValue::Lines(values) => values.join("\n"),
            };
            let (entry, entry_items) = prepare_entry(key.clone(), source, entry_index, None);
            entries.push(entry);
            items.extend(entry_items);
        }
        lang = Some(map);
    } else {
        let mut entry_index = 0;
        for file in chapters::files(&q) {
            for segment in chapters::extract(&file)? {
                let (entry, entry_items) = prepare_entry(
                    segment.cache_id.clone(),
                    segment.source.clone(),
                    entry_index,
                    None,
                );
                entries.push(entry);
                items.extend(entry_items);
                chapter_segs.push(segment);
                entry_index += 1;
            }
        }
    }
    if document.meta.total_entries != entries.len()
        || document.meta.source_fingerprint != source_fingerprint(&entries)
    {
        return Err("任务书内容在 CMP 生成后发生了变化，请重新扫描并翻译".into());
    }
    if document.records.len() != items.len() {
        return Err("CMP 翻译条目数量与当前任务书不一致".into());
    }

    let expected = items
        .iter()
        .map(|item| ((item.entry_id.as_str(), item.path.as_str()), item))
        .collect::<HashMap<_, _>>();
    let mut unit_targets = HashMap::new();
    let mut pending_reviews = HashSet::new();
    for record in &document.records {
        let item = expected
            .get(&(record.entry_id.as_str(), record.path.as_str()))
            .ok_or_else(|| format!("CMP 包含未知回填位置：{} {}", record.entry_id, record.path))?;
        let expected_file = entry_source_file(m, &record.entry_id);
        if record.file != expected_file {
            return Err(format!(
                "CMP 文件归属被修改：{} 应属于 {}",
                record.entry_id, expected_file
            ));
        }
        if record.source != item.source {
            return Err(format!(
                "CMP 英文原文被修改：{} {}。只允许修改箭头右侧中文",
                record.entry_id, record.path
            ));
        }
        if record.target.trim().is_empty() {
            return Err(format!(
                "CMP 译文不能为空：{} {}",
                record.entry_id, record.path
            ));
        }
        let problems = warnings(&record.source, &record.target);
        if !problems.is_empty() {
            return Err(format!(
                "CMP 译文未通过格式守卫：{} {}（{}）",
                record.entry_id,
                record.path,
                problems.join("；")
            ));
        }
        if record.status != "translated" && record.target == record.source {
            pending_reviews.insert(record.entry_id.clone());
        }
        unit_targets.insert(item.id.clone(), record.target.clone());
    }
    if unit_targets.len() != items.len() {
        return Err("CMP 缺少一个或多个回填位置".into());
    }

    let mut results = HashMap::new();
    let mut report_warnings = BTreeMap::new();
    let mut details = BTreeMap::new();
    for entry in &entries {
        if let EntryKind::Untouched(reason) = &entry.kind {
            results.insert(entry.id.clone(), entry.source.clone());
            report_warnings.insert(entry.id.clone(), vec![reason.clone()]);
            details.insert(
                entry.id.clone(),
                json!({"source":entry.source,"failed":entry.source}),
            );
            continue;
        }
        let target = render_entry(entry, &unit_targets)?;
        let problems = warnings(&entry.source, &target);
        if !problems.is_empty() {
            return Err(format!(
                "CMP 回填后的完整条目未通过格式守卫：{}（{}）",
                entry.id,
                problems.join("；")
            ));
        }
        if pending_reviews.contains(&entry.id) {
            report_warnings.insert(
                entry.id.clone(),
                vec!["机器翻译失败或未通过格式守卫，CMP 仍保留英文原文".into()],
            );
            details.insert(
                entry.id.clone(),
                json!({"source":entry.source,"failed":target}),
            );
        }
        results.insert(entry.id.clone(), target);
    }

    let mut pending_outputs = Vec::new();
    let (source_file, target_file) = if let Some(mut map) = lang {
        for (key, value) in &mut map {
            if let Some(target) = results.get(key) {
                *value = match value {
                    LangValue::Text(_) => LangValue::Text(target.clone()),
                    LangValue::Lines(_) => {
                        LangValue::Lines(target.split('\n').map(str::to_string).collect())
                    }
                };
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
        for segment in chapter_segs {
            by_file
                .entry(segment.path)
                .or_default()
                .push((segment.index, results[&segment.cache_id].clone()));
        }
        for (file, replacements) in by_file {
            let (content, _) = chapters::render_replacements(&file, &replacements)?;
            pending_outputs.push(FileOutput {
                archive_name: format!("chapters/{}", file.file_name().unwrap().to_string_lossy()),
                path: file,
                content,
            });
        }
        (q.join("chapters"), q.join("chapters"))
    };

    logging::info(
        "translation",
        "cmp_validation_completed",
        "CMP 校对文件及最终输出已通过安全校验",
        json!({
            "task_id":task_id,
            "cmp_path":cmp_path,
            "entries":entries.len(),
            "translation_units":document.records.len(),
            "output_files":pending_outputs.len()
        }),
    );
    phase.set("backup");
    logging::info(
        "translation",
        "backup_started",
        "开始备份当前任务书",
        json!({"task_id":task_id,"quests_dir":q,"mode":m}),
    );
    let backup = backup(&q, m)?;
    logging::info(
        "translation",
        "backup_completed",
        "当前任务书备份完成",
        json!({"task_id":task_id,"backup_dir":backup}),
    );
    phase.set("commit");
    logging::info(
        "translation",
        "output_commit_started",
        "开始提交全部翻译输出",
        json!({"task_id":task_id,"output_files":pending_outputs.len()}),
    );
    commit_outputs(&pending_outputs, task_id)?;
    logging::info(
        "translation",
        "output_commit_completed",
        "全部翻译输出已提交",
        json!({"task_id":task_id,"output_files":pending_outputs.len()}),
    );
    phase.set("metadata");
    let outputs = pending_outputs
        .into_iter()
        .map(|output| (output.archive_name, output.content, json!({})))
        .collect::<Vec<_>>();
    let settings = Settings {
        provider: document.meta.provider.clone(),
        base_url: document.meta.base_url.clone(),
        model: document.meta.model.clone(),
        style: document.meta.style.clone(),
        glossary_enabled: document.meta.glossary_enabled,
        glossary_fingerprint: document.meta.glossary_fingerprint.clone(),
        ..Settings::default()
    };
    let mut cache = load_cache(&q);
    for entry in &entries {
        let target = results.get(&entry.id).unwrap_or(&entry.source);
        if target != &entry.source {
            cache.insert(cache_key(&entry.source, &settings), target.clone());
        }
    }
    if let Err(error) = save_cache(&q, &cache) {
        logging::warn(
            "translation",
            "cache_update_failed_after_commit",
            "任务书已写入，但翻译缓存更新失败",
            json!({"task_id":task_id,"quests_dir":q,"error":error}),
        );
    }
    let report = Report {
        source_file: source_file.display().to_string(),
        target_file: target_file.display().to_string(),
        backup_dir: backup.display().to_string(),
        total_entries: entries.len(),
        translated_entries: entries.len().saturating_sub(report_warnings.len()),
        cache_hits: document.meta.cache_hits,
        failed_entries: Vec::new(),
        warnings: report_warnings,
        failed_translations: details,
    };
    let report_value = serde_json::to_value(&report).map_err(|e| e.to_string())?;
    let run_id = match History::new(data_dir)
        .and_then(|history| history.insert(&q, m, &settings, &report_value, &outputs))
    {
        Ok(run_id) => run_id,
        Err(error) => {
            logging::warn(
                "translation",
                "history_write_failed_after_commit",
                "任务书已写入，但翻译历史保存失败",
                json!({"task_id":task_id,"quests_dir":q,"error":error}),
            );
            0
        }
    };
    let report_path = q.join(".ftb-translater/report-latest.json");
    let report_result = serde_json::to_vec_pretty(&report_value)
        .map_err(|error| error.to_string())
        .and_then(|bytes| fs::write(&report_path, bytes).map_err(|error| error.to_string()));
    if let Err(error) = report_result {
        logging::warn(
            "translation",
            "report_write_failed_after_commit",
            "任务书已写入，但最新报告保存失败",
            json!({"task_id":task_id,"path":report_path,"error":error}),
        );
    }
    logging::info(
        "translation",
        "cmp_applied",
        "CMP 校对结果已写入任务书",
        json!({"task_id":task_id,"cmp_path":cmp_path,"run_id":run_id,"output_files":outputs.len()}),
    );
    Ok(json!({"report":report,"run_id":run_id,"task_id":task_id}))
}

fn cmp_task_id(path: &Path) -> String {
    cmp::load(path)
        .ok()
        .map(|document| document.meta.task_id)
        .filter(|task_id| !task_id.trim().is_empty())
        .unwrap_or_else(logging::task_id)
}

pub fn export_cmp(payload: &Value) -> Result<Value, String> {
    let source = Path::new(payload["cmp_path"].as_str().ok_or("缺少 CMP 文件路径")?);
    let target = Path::new(payload["path"].as_str().ok_or("缺少 CMP 导出路径")?);
    let task_id = cmp_task_id(source);
    cmp::export(source, target)?;
    logging::info(
        "translation",
        "cmp_exported",
        "CMP 校对文件已导出",
        json!({"task_id":task_id,"source":source,"destination":target}),
    );
    Ok(json!({"path":target}))
}

pub fn review_cmp(payload: &Value) -> Result<Value, String> {
    let path = Path::new(payload["cmp_path"].as_str().ok_or("缺少 CMP 文件路径")?);
    let document = cmp::load(path)?;
    Ok(json!({
        "entries": document.records.iter().enumerate().map(|(index, record)| json!({
            "index": index,
            "entry_id": record.entry_id,
            "path": record.path,
            "file": record.file,
            "source": record.source,
            "target": record.target,
            "status": record.status,
        })).collect::<Vec<_>>(),
    }))
}

pub fn save_cmp_edits(payload: &Value) -> Result<Value, String> {
    let path = Path::new(payload["cmp_path"].as_str().ok_or("缺少 CMP 文件路径")?);
    let edits = payload["entries"]
        .as_array()
        .ok_or("缺少 CMP 校对表格内容")?;
    let mut document = cmp::load(path)?;
    if edits.len() != document.records.len() {
        return Err("CMP 校对表格条目数已变化，请重新打开校对表格".into());
    }
    for (index, (record, edit)) in document.records.iter_mut().zip(edits).enumerate() {
        if edit["index"].as_u64() != Some(index as u64)
            || edit["entry_id"].as_str() != Some(record.entry_id.as_str())
            || edit["path"].as_str() != Some(record.path.as_str())
            || edit["source"].as_str() != Some(record.source.as_str())
        {
            return Err("CMP 校对表格内容已过期或不属于当前文件，请重新打开".into());
        }
        let target = edit["target"].as_str().ok_or("校对译文必须是文本")?;
        if target.trim().is_empty() {
            return Err(format!("第 {} 条译文不能为空", index + 1));
        }
        record.target = target.to_string();
    }
    cmp::write(path, &document)?;
    logging::info(
        "translation",
        "cmp_edits_saved",
        "CMP 校对表格修改已保存",
        json!({"task_id":document.meta.task_id,"cmp_path":path,"entries":edits.len()}),
    );
    Ok(json!({"saved":true,"entries":edits.len()}))
}

pub fn open_cmp(payload: &Value) -> Result<Value, String> {
    let path = PathBuf::from(payload["cmp_path"].as_str().ok_or("缺少 CMP 文件路径")?);
    if path.extension().is_none_or(|extension| extension != "cmp") || !path.is_file() {
        return Err("CMP 文件不存在或扩展名无效".into());
    }
    let task_id = cmp_task_id(&path);
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg("-t");
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = Command::new("notepad");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command
        .arg(&path)
        .spawn()
        .map_err(|e| format!("无法打开 CMP 文件 {}：{e}", path.display()))?;
    logging::info(
        "translation",
        "cmp_opened",
        "CMP 校对文件已交给文本编辑器打开",
        json!({"task_id":task_id,"cmp_path":path}),
    );
    Ok(json!({"opened":true}))
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
        assert_eq!(restore(&p, &t).unwrap(), src);
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
    fn rejects_injected_or_duplicated_opaque_placeholders() {
        let (protected, tokens) = protect("Use minecraft:stone and 16 blocks");
        assert!(restore(&format!("{protected} ⟨P_999⟩"), &tokens).is_err());
        assert!(restore(&format!("{protected} {}", tokens[0].0), &tokens).is_err());
    }
    #[test]
    fn protects_resource_ids_selectors_tags_and_numbers() {
        let source = "Give 16 minecraft:stone to @p[tag=builder] from #forge:storage_blocks/iron";
        let (protected, tokens) = protect(source);
        assert!(!protected.contains("minecraft:stone"));
        assert!(!protected.contains("@p[tag=builder]"));
        assert!(!protected.contains("#forge:storage_blocks/iron"));
        assert!(!protected.contains("16"));
        assert_eq!(restore(&protected, &tokens).unwrap(), source);
    }
    #[test]
    fn rejects_changed_json_shape() {
        let a = r#"{"text":"Hello","color":"red"}"#;
        let b = r#"{"text":"你好","color":"blue"}"#;
        assert!(!warnings(a, b).is_empty())
    }
    #[test]
    fn accepts_only_rich_text_display_field_changes() {
        let source = r#"{"translate":"key.example","with":["Now"],"clickEvent":{"action":"run_command","value":"/say hi"}}"#;
        let translated = r#"{"translate":"key.example","with":["现在"],"clickEvent":{"action":"run_command","value":"/say hi"}}"#;
        let changed_command = r#"{"translate":"key.example","with":["现在"],"clickEvent":{"action":"run_command","value":"/say 你好"}}"#;
        assert!(warnings(source, translated).is_empty());
        assert!(!warnings(source, changed_command).is_empty());
    }
    #[test]
    fn duplicate_key_json_is_never_sent_as_plain_translation_text() {
        let source = r#"{"text":"First","text":"Second"}"#.to_string();
        let (entry, items) = prepare_entry("duplicate".into(), source.clone(), 0, None);
        assert!(items.is_empty());
        assert!(matches!(&entry.kind, EntryKind::Untouched(_)));
        assert_eq!(render_entry(&entry, &HashMap::new()).unwrap(), source);
    }
    #[test]
    fn glossary_is_optional_and_restores_curated_terms() {
        let source = "Use Mekanism with an Enchanting Table";
        let (plain, plain_tokens) = protect_for_translation(source, None);
        assert_eq!(restore(&plain, &plain_tokens).unwrap(), source);
        assert!(!plain.contains("⟨G_"));

        let d = tempdir().unwrap();
        let path = glossary::ensure_default(d.path()).unwrap();
        let loaded = glossary::Loaded::load(&path).unwrap();
        let (protected, tokens) = protect_for_translation(source, Some(&loaded));
        assert_eq!(protected.matches("⟨G_").count(), 2);
        assert_eq!(
            restore(&protected, &tokens).unwrap(),
            "Use 通用机械 with an 附魔台"
        );
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
    fn rich_text_cache_does_not_reuse_legacy_whole_json_results() {
        let settings = Settings {
            provider: providers::OPENAI_COMPATIBLE.into(),
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-chat".into(),
            ..Settings::default()
        };
        let source = r#"{"text":"Open guide","color":"gold"}"#;
        let mut legacy = Sha256::new();
        legacy.update(
            json!({
                "source_text":source,
                "model":"deepseek-chat",
                "target_locale":"zh_cn",
                "style":settings.style
            })
            .to_string(),
        );
        assert_ne!(cache_key(source, &settings), hex::encode(legacy.finalize()));
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

    fn write_test_cmp(
        path: &Path,
        quests: &Path,
        source: &str,
        recorded_source: &str,
        target: &str,
    ) {
        let (entry, _) = prepare_entry("title".into(), source.into(), 0, None);
        cmp::write(
            path,
            &cmp::Document {
                meta: cmp::Meta {
                    version: 1,
                    task_id: "test-task".into(),
                    quests_dir: quests.display().to_string(),
                    mode: "lang".into(),
                    source_fingerprint: source_fingerprint(&[entry]),
                    provider: providers::GOOGLE_WEB.into(),
                    base_url: "https://translate.googleapis.com".into(),
                    model: "google-web".into(),
                    style: "自然中文".into(),
                    glossary_enabled: false,
                    glossary_fingerprint: String::new(),
                    total_entries: 1,
                    cache_hits: 0,
                },
                records: vec![cmp::Record {
                    file: "lang/en_us.snbt".into(),
                    entry_id: "title".into(),
                    path: "$".into(),
                    source: recorded_source.into(),
                    target: target.into(),
                    status: "translated".into(),
                }],
            },
        )
        .unwrap();
    }

    #[test]
    fn retry_validation_rejects_another_pack_or_modified_source() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("pack-a/quests");
        let other = directory.path().join("pack-b/quests");
        fs::create_dir_all(quests.join("lang")).unwrap();
        fs::create_dir_all(other.join("lang")).unwrap();
        let cmp_path = directory.path().join("review.cmp");
        write_test_cmp(&cmp_path, &quests, "Hello", "Hello", "Hello");
        let mut document = cmp::load(&cmp_path).unwrap();
        let (entry, items) = prepare_entry("title".into(), "Hello".into(), 0, None);

        validate_cmp_source(
            &document,
            &quests,
            "lang",
            std::slice::from_ref(&entry),
            &items,
        )
        .unwrap();
        assert!(validate_cmp_source(
            &document,
            &other,
            "lang",
            std::slice::from_ref(&entry),
            &items
        )
        .is_err());

        document.records[0].source = "Changed".into();
        assert!(validate_cmp_source(
            &document,
            &quests,
            "lang",
            std::slice::from_ref(&entry),
            &items
        )
        .is_err());
    }

    #[test]
    fn cmp_is_validated_before_it_writes_the_language_file() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("config/ftbquests/quests");
        fs::create_dir_all(quests.join("lang")).unwrap();
        fs::write(quests.join("lang/en_us.snbt"), r#"{ title: "Hello" }"#).unwrap();
        let cmp_path = directory.path().join("review.cmp");
        write_test_cmp(&cmp_path, &quests, "Hello", "Hello", "你好");

        assert!(!quests.join("lang/zh_cn.snbt").exists());
        let result = apply_cmp(
            &directory.path().join("app-data"),
            &json!({"cmp_path":cmp_path,"quests_dir":quests.display().to_string()}),
        )
        .unwrap();
        assert_eq!(result["report"]["translated_entries"], 1);
        let translated = snbt::load(&quests.join("lang/zh_cn.snbt")).unwrap();
        assert_eq!(translated[0].1, LangValue::Text("你好".into()));
    }

    #[test]
    fn history_failure_after_commit_does_not_report_writeback_failure() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("config/ftbquests/quests");
        fs::create_dir_all(quests.join("lang")).unwrap();
        fs::write(quests.join("lang/en_us.snbt"), r#"{ title: "Hello" }"#).unwrap();
        let cmp_path = directory.path().join("review.cmp");
        write_test_cmp(&cmp_path, &quests, "Hello", "Hello", "你好");
        let unusable_data_dir = directory.path().join("app-data-file");
        fs::write(&unusable_data_dir, "not a directory").unwrap();

        let result = apply_cmp(
            &unusable_data_dir,
            &json!({"cmp_path":cmp_path,"quests_dir":quests.display().to_string()}),
        )
        .unwrap();

        assert_eq!(result["run_id"], 0);
        let translated = snbt::load(&quests.join("lang/zh_cn.snbt")).unwrap();
        assert_eq!(translated[0].1, LangValue::Text("你好".into()));
    }

    #[test]
    fn cmp_with_modified_english_never_writes_output() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("config/ftbquests/quests");
        fs::create_dir_all(quests.join("lang")).unwrap();
        fs::write(quests.join("lang/en_us.snbt"), r#"{ title: "Hello" }"#).unwrap();
        let cmp_path = directory.path().join("review.cmp");
        write_test_cmp(&cmp_path, &quests, "Hello", "Changed", "你好");

        assert!(apply_cmp(
            &directory.path().join("app-data"),
            &json!({"cmp_path":cmp_path,"quests_dir":quests.display().to_string()})
        )
        .is_err());
        assert!(!quests.join("lang/zh_cn.snbt").exists());
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

        assert!(commit_outputs(&outputs, "test-task").is_err());
        assert_eq!(fs::read_to_string(first).unwrap(), "original");
    }

    #[test]
    fn backups_created_in_the_same_second_never_overwrite_each_other() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("quests");
        fs::create_dir_all(quests.join("lang")).unwrap();
        fs::write(quests.join("lang/en_us.snbt"), "{a: \"A\"}").unwrap();

        let first = backup(&quests, "lang").unwrap();
        let second = backup(&quests, "lang").unwrap();

        assert_ne!(first, second);
        assert!(first.join("lang/en_us.snbt").is_file());
        assert!(second.join("lang/en_us.snbt").is_file());
    }
}
