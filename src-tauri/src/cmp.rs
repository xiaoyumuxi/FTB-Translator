use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, path::Path};

const HEADER: &str = "# FTB Translater CMP v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Meta {
    pub version: u32,
    #[serde(default)]
    pub task_id: String,
    pub quests_dir: String,
    pub mode: String,
    pub source_fingerprint: String,
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub style: String,
    pub glossary_enabled: bool,
    pub glossary_fingerprint: String,
    pub total_entries: usize,
    pub cache_hits: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Record {
    pub file: String,
    pub entry_id: String,
    pub path: String,
    pub source: String,
    pub target: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub meta: Meta,
    pub records: Vec<Record>,
}

#[derive(Serialize, Deserialize)]
struct Location {
    file: String,
    entry_id: String,
    path: String,
    status: String,
}

pub fn write(path: &Path, document: &Document) -> Result<(), String> {
    let mut output = String::new();
    output.push_str(HEADER);
    output.push('\n');
    output.push_str("# 只修改箭头右侧的中文；保留 @ 行、英文原文、引号与 JSON 转义。\n");
    output.push_str("# meta ");
    output.push_str(&serde_json::to_string(&document.meta).map_err(|e| e.to_string())?);
    output.push_str("\n\n");
    let mut current_file = None;
    for record in &document.records {
        if current_file != Some(record.file.as_str()) {
            output.push_str("## file ");
            output.push_str(&serde_json::to_string(&record.file).map_err(|e| e.to_string())?);
            output.push_str("\n\n");
            current_file = Some(record.file.as_str());
        }
        output.push_str("@ ");
        output.push_str(
            &serde_json::to_string(&Location {
                file: record.file.clone(),
                entry_id: record.entry_id.clone(),
                path: record.path.clone(),
                status: record.status.clone(),
            })
            .map_err(|e| e.to_string())?,
        );
        output.push('\n');
        output.push_str(&serde_json::to_string(&record.source).map_err(|e| e.to_string())?);
        output.push_str(" -> ");
        output.push_str(&serde_json::to_string(&record.target).map_err(|e| e.to_string())?);
        output.push_str("\n\n");
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, output).map_err(|e| format!("无法保存 CMP 校对文件：{e}"))
}

pub fn load(path: &Path) -> Result<Document, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("无法读取 CMP 校对文件 {}：{e}", path.display()))?;
    parse(&content)
}

pub fn parse(content: &str) -> Result<Document, String> {
    let mut lines = content.lines().enumerate().peekable();
    let (_, header) = lines.next().ok_or("CMP 文件为空")?;
    if header.trim_start_matches('\u{feff}') != HEADER {
        return Err("CMP 文件头无效或版本不受支持".into());
    }
    let mut meta = None;
    let mut records = Vec::new();
    let mut locations = HashSet::new();
    while let Some((line_index, line)) = lines.next() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(raw) = line.strip_prefix("# meta ") {
            if meta.is_some() {
                return Err(format!("CMP 第 {} 行重复定义 meta", line_index + 1));
            }
            let value = serde_json::from_str::<Meta>(raw)
                .map_err(|e| format!("CMP 第 {} 行 meta 无效：{e}", line_index + 1))?;
            if value.version != 1 {
                return Err(format!("不支持 CMP 版本 {}", value.version));
            }
            meta = Some(value);
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let raw_location = line
            .strip_prefix("@ ")
            .ok_or_else(|| format!("CMP 第 {} 行缺少 @ 回填位置", line_index + 1))?;
        let location = serde_json::from_str::<Location>(raw_location)
            .map_err(|e| format!("CMP 第 {} 行回填位置无效：{e}", line_index + 1))?;
        if !locations.insert((location.entry_id.clone(), location.path.clone())) {
            return Err(format!("CMP 第 {} 行重复定义同一回填位置", line_index + 1));
        }
        let (pair_index, pair) = loop {
            let next = lines
                .next()
                .ok_or_else(|| format!("CMP 第 {} 行缺少英文 -> 中文", line_index + 1))?;
            if !next.1.trim().is_empty() && !next.1.trim().starts_with('#') {
                break next;
            }
        };
        let (source, target) = parse_pair(pair.trim())
            .map_err(|e| format!("CMP 第 {} 行无效：{e}", pair_index + 1))?;
        records.push(Record {
            file: location.file,
            entry_id: location.entry_id,
            path: location.path,
            source,
            target,
            status: location.status,
        });
    }
    let meta = meta.ok_or("CMP 文件缺少 meta")?;
    if records.is_empty() {
        return Err("CMP 文件没有翻译条目".into());
    }
    Ok(Document { meta, records })
}

fn parse_pair(line: &str) -> Result<(String, String), String> {
    let mut stream = serde_json::Deserializer::from_str(line).into_iter::<String>();
    let source = stream
        .next()
        .ok_or("缺少英文原文")?
        .map_err(|e| format!("英文原文不是有效 JSON 字符串：{e}"))?;
    let offset = stream.byte_offset();
    let target = line[offset..]
        .strip_prefix(" -> ")
        .ok_or("英文和中文之间必须使用空格、->、空格")?;
    let target = serde_json::from_str::<String>(target)
        .map_err(|e| format!("中文译文不是有效 JSON 字符串：{e}"))?;
    Ok((source, target))
}

pub fn export(source: &Path, target: &Path) -> Result<(), String> {
    load(source)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(source, target)
        .map(|_| ())
        .map_err(|e| format!("无法导出 CMP 校对文件：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn document() -> Document {
        Document {
            meta: Meta {
                version: 1,
                task_id: "20260714T120000.000Z-0001".into(),
                quests_dir: "/pack/quests".into(),
                mode: "lang".into(),
                source_fingerprint: "abc".into(),
                provider: "google_web".into(),
                base_url: "https://translate.googleapis.com".into(),
                model: "google-web".into(),
                style: "自然中文".into(),
                glossary_enabled: false,
                glossary_fingerprint: String::new(),
                total_entries: 1,
                cache_hits: 0,
            },
            records: vec![Record {
                file: "lang/en_us.snbt".into(),
                entry_id: "quest.example".into(),
                path: "/text".into(),
                source: "Open -> guide\nnow".into(),
                target: "立即打开指南".into(),
                status: "translated".into(),
            }],
        }
    }

    #[test]
    fn roundtrips_human_readable_cmp_with_arrows_and_newlines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("review.cmp");
        let expected = document();
        write(&path, &expected).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(r#""Open -> guide\nnow" -> "立即打开指南""#));
        assert_eq!(load(&path).unwrap(), expected);
    }

    #[test]
    fn rejects_duplicate_locations_and_broken_pairs() {
        let mut content = String::from(HEADER);
        content.push_str("\n# meta ");
        content.push_str(&serde_json::to_string(&document().meta).unwrap());
        content.push_str("\n@ {\"file\":\"lang/en_us.snbt\",\"entry_id\":\"a\",\"path\":\"$\",\"status\":\"translated\"}\n");
        content.push_str("\"A\" -> \"甲\"\n");
        content.push_str("@ {\"file\":\"lang/en_us.snbt\",\"entry_id\":\"a\",\"path\":\"$\",\"status\":\"translated\"}\n");
        content.push_str("\"A\" -> \"乙\"\n");
        assert!(parse(&content).is_err());
        assert!(parse_pair(r#""A -> B" => "甲""#).is_err());
    }

    #[test]
    fn metadata_is_json_and_never_contains_arbitrary_values() {
        let value = serde_json::to_value(&document().meta).unwrap();
        assert_eq!(value["version"], serde_json::Value::from(1));
    }

    #[test]
    fn older_v1_metadata_without_task_id_remains_readable() {
        let mut value = serde_json::to_value(&document().meta).unwrap();
        value.as_object_mut().unwrap().remove("task_id");
        let parsed: Meta = serde_json::from_value(value).unwrap();
        assert!(parsed.task_id.is_empty());
    }
}
