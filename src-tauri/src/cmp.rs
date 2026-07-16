use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, path::Path};

const HEADER: &str = "# FTB Translator CMP v1";
const LEGACY_HEADER: &str = "# FTB Translater CMP v1";

fn invalid(message: impl Into<String>) -> AppError {
    let message = message.into();
    AppError::cmp_invalid(message.clone(), message)
}

fn invalid_with(user_message: impl Into<String>, internal_message: impl Into<String>) -> AppError {
    AppError::cmp_invalid(user_message, internal_message)
}

fn supported_status(status: &str) -> bool {
    matches!(
        status,
        "translated"
            | "unchanged"
            | "review"
            | "rate_limited"
            | "request_failed"
            | "format_guard"
            | "fallback"
    )
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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

fn hash_field(hash: &mut Sha256, value: &str) {
    hash.update((value.len() as u64).to_le_bytes());
    hash.update(value.as_bytes());
}

/// Covers every CMP field that a human editor is not allowed to change.
/// The target text is deliberately excluded so normal review edits remain valid.
pub fn protected_fingerprint(document: &Document) -> String {
    let mut hash = Sha256::new();
    hash.update(b"ftb-translator-cmp-protected-v1\0");
    hash_field(
        &mut hash,
        &serde_json::to_string(&document.meta)
            .expect("CMP metadata contains only values supported by serde_json"),
    );
    let mut records = document.records.iter().collect::<Vec<_>>();
    records.sort_by(|left, right| left.file.cmp(&right.file));
    for record in records {
        for value in [
            record.file.as_str(),
            record.entry_id.as_str(),
            record.path.as_str(),
            record.source.as_str(),
            record.status.as_str(),
        ] {
            hash_field(&mut hash, value);
        }
    }
    hex::encode(hash.finalize())
}

fn content_revision(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Location {
    file: String,
    entry_id: String,
    path: String,
    status: String,
}

pub fn write(path: &Path, document: &Document) -> AppResult<()> {
    validate_document(document)?;
    let mut output = String::new();
    output.push_str(HEADER);
    output.push('\n');
    output.push_str("# 只修改箭头右侧的中文；保留 @ 行、英文原文、引号与 JSON 转义。\n");
    output.push_str("# meta ");
    let mut metadata = serde_json::to_string(&document.meta)
        .map_err(|error| invalid_with(error.to_string(), error.to_string()))?;
    metadata.pop();
    metadata.push_str(",\"protected_hash\":");
    metadata.push_str(
        &serde_json::to_string(&protected_fingerprint(document))
            .map_err(|error| invalid_with(error.to_string(), error.to_string()))?,
    );
    metadata.push('}');
    output.push_str(&metadata);
    output.push_str("\n\n");
    // CMP file sections are presentation-only, but keeping them in a canonical order
    // makes repeated saves stable. Rust's slice sort is stable, so source order within
    // each file remains unchanged.
    let mut records = document.records.iter().collect::<Vec<_>>();
    records.sort_by(|left, right| left.file.cmp(&right.file));
    let mut current_file = None;
    for record in records {
        if current_file != Some(record.file.as_str()) {
            output.push_str("## file ");
            output.push_str(
                &serde_json::to_string(&record.file)
                    .map_err(|error| invalid_with(error.to_string(), error.to_string()))?,
            );
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
            .map_err(|error| invalid_with(error.to_string(), error.to_string()))?,
        );
        output.push('\n');
        output.push_str(
            &serde_json::to_string(&record.source)
                .map_err(|error| invalid_with(error.to_string(), error.to_string()))?,
        );
        output.push_str(" -> ");
        output.push_str(
            &serde_json::to_string(&record.target)
                .map_err(|error| invalid_with(error.to_string(), error.to_string()))?,
        );
        output.push_str("\n\n");
    }
    crate::atomic_file::write(path, output)
        .map_err(|error| invalid_with(format!("无法保存 CMP 校对文件：{error}"), error.to_string()))
}

fn validate_document(document: &Document) -> AppResult<()> {
    if document.meta.version != 1 {
        return Err(invalid(format!(
            "不支持 CMP 版本 {}",
            document.meta.version
        )));
    }
    if document.records.is_empty() {
        return Err(invalid("CMP 文件没有翻译条目"));
    }
    let mut locations = HashSet::new();
    for record in &document.records {
        if !supported_status(&record.status) {
            return Err(invalid(format!("CMP 包含不支持的状态：{}", record.status)));
        }
        if !locations.insert((record.entry_id.as_str(), record.path.as_str())) {
            return Err(invalid("CMP 重复定义同一回填位置"));
        }
    }
    Ok(())
}

pub fn load(path: &Path) -> AppResult<Document> {
    load_with_revision(path).map(|(document, _)| document)
}

pub fn load_with_revision(path: &Path) -> AppResult<(Document, String)> {
    let bytes = fs::read(path).map_err(|error| {
        invalid_with(
            format!("无法读取 CMP 校对文件 {}：{error}", path.display()),
            error.to_string(),
        )
    })?;
    let content = std::str::from_utf8(&bytes).map_err(|error| {
        invalid_with(
            format!("CMP 校对文件不是有效 UTF-8：{error}"),
            error.to_string(),
        )
    })?;
    parse(content).map(|document| (document, content_revision(&bytes)))
}

pub fn revision(path: &Path) -> AppResult<String> {
    fs::read(path)
        .map(|bytes| content_revision(&bytes))
        .map_err(|error| {
            invalid_with(
                format!("无法读取 CMP 校对文件 {}：{error}", path.display()),
                error.to_string(),
            )
        })
}

pub fn parse(content: &str) -> AppResult<Document> {
    let mut lines = content.lines().enumerate().peekable();
    let (_, header) = lines.next().ok_or_else(|| invalid("CMP 文件为空"))?;
    let header = header.trim_start_matches('\u{feff}');
    if header != HEADER && header != LEGACY_HEADER {
        return Err(invalid("CMP 文件头无效或版本不受支持"));
    }
    let mut meta = None;
    let mut protected_hash = None;
    let mut records = Vec::new();
    let mut locations = HashSet::new();
    let mut current_file = None;
    while let Some((line_index, line)) = lines.next() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(raw) = line.strip_prefix("# meta ") {
            if meta.is_some() {
                return Err(invalid(format!(
                    "CMP 第 {} 行重复定义 meta",
                    line_index + 1
                )));
            }
            let mut raw_value = serde_json::from_str::<Value>(raw).map_err(|error| {
                invalid_with(
                    format!("CMP 第 {} 行 meta 无效：{error}", line_index + 1),
                    error.to_string(),
                )
            })?;
            let object = raw_value.as_object_mut().ok_or_else(|| {
                invalid(format!(
                    "CMP 第 {} 行 meta 必须是 JSON 对象",
                    line_index + 1
                ))
            })?;
            protected_hash = object
                .remove("protected_hash")
                .map(|value| {
                    value.as_str().map(str::to_string).ok_or_else(|| {
                        invalid(format!(
                            "CMP 第 {} 行 protected_hash 必须是字符串",
                            line_index + 1
                        ))
                    })
                })
                .transpose()?;
            let value = serde_json::from_value::<Meta>(raw_value).map_err(|error| {
                invalid_with(
                    format!("CMP 第 {} 行 meta 无效：{error}", line_index + 1),
                    error.to_string(),
                )
            })?;
            if value.version != 1 {
                return Err(invalid(format!("不支持 CMP 版本 {}", value.version)));
            }
            meta = Some(value);
            continue;
        }
        if let Some(raw) = line.strip_prefix("## file ") {
            current_file = Some(serde_json::from_str::<String>(raw).map_err(|error| {
                invalid_with(
                    format!("CMP 第 {} 行文件分组无效：{error}", line_index + 1),
                    error.to_string(),
                )
            })?);
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let raw_location = line
            .strip_prefix("@ ")
            .ok_or_else(|| invalid(format!("CMP 第 {} 行缺少 @ 回填位置", line_index + 1)))?;
        let location = serde_json::from_str::<Location>(raw_location).map_err(|error| {
            invalid_with(
                format!("CMP 第 {} 行回填位置无效：{error}", line_index + 1),
                error.to_string(),
            )
        })?;
        if current_file
            .as_deref()
            .is_some_and(|file| file != location.file)
        {
            return Err(invalid(format!(
                "CMP 第 {} 行的文件归属与当前 ## file 分组不一致",
                line_index + 1
            )));
        }
        if !supported_status(&location.status) {
            return Err(invalid(format!(
                "CMP 第 {} 行包含不支持的状态：{}",
                line_index + 1,
                location.status
            )));
        }
        if !locations.insert((location.entry_id.clone(), location.path.clone())) {
            return Err(invalid(format!(
                "CMP 第 {} 行重复定义同一回填位置",
                line_index + 1
            )));
        }
        let (pair_index, pair) = loop {
            let next = lines
                .next()
                .ok_or_else(|| invalid(format!("CMP 第 {} 行缺少英文 -> 中文", line_index + 1)))?;
            if !next.1.trim().is_empty() && !next.1.trim().starts_with('#') {
                break next;
            }
        };
        let (source, target) = parse_pair(pair.trim()).map_err(|error| {
            invalid_with(
                format!("CMP 第 {} 行无效：{error}", pair_index + 1),
                error.internal_message,
            )
        })?;
        records.push(Record {
            file: location.file,
            entry_id: location.entry_id,
            path: location.path,
            source,
            target,
            status: location.status,
        });
    }
    let meta = meta.ok_or_else(|| invalid("CMP 文件缺少 meta"))?;
    if records.is_empty() {
        return Err(invalid("CMP 文件没有翻译条目"));
    }
    let document = Document { meta, records };
    validate_document(&document)?;
    if let Some(expected) = protected_hash {
        let actual = protected_fingerprint(&document);
        if expected != actual {
            return Err(invalid_with(
                "CMP 的受保护内容已被修改；只允许编辑箭头右侧译文",
                format!("protected hash mismatch: expected={expected} actual={actual}"),
            ));
        }
    }
    Ok(document)
}

fn parse_pair(line: &str) -> AppResult<(String, String)> {
    let mut stream = serde_json::Deserializer::from_str(line).into_iter::<String>();
    let source = stream
        .next()
        .ok_or_else(|| invalid("缺少英文原文"))?
        .map_err(|error| {
            invalid_with(
                format!("英文原文不是有效 JSON 字符串：{error}"),
                error.to_string(),
            )
        })?;
    let offset = stream.byte_offset();
    let target = line[offset..]
        .strip_prefix(" -> ")
        .ok_or_else(|| invalid("英文和中文之间必须使用空格、->、空格"))?;
    let target = serde_json::from_str::<String>(target).map_err(|error| {
        invalid_with(
            format!("中文译文不是有效 JSON 字符串：{error}"),
            error.to_string(),
        )
    })?;
    Ok((source, target))
}

pub fn export(source: &Path, target: &Path) -> AppResult<()> {
    load(source)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| invalid_with(error.to_string(), error.to_string()))?;
    }
    fs::copy(source, target)
        .map(|_| ())
        .map_err(|error| invalid_with(format!("无法导出 CMP 校对文件：{error}"), error.to_string()))
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
    fn reads_cmp_files_written_before_the_project_rename() {
        let meta = serde_json::to_string(&document().meta).unwrap();
        let content = format!(
            "{LEGACY_HEADER}\n# meta {meta}\n@ {{\"file\":\"lang/en_us.snbt\",\"entry_id\":\"quest.example\",\"path\":\"/text\",\"status\":\"translated\"}}\n\"Open -> guide\\nnow\" -> \"立即打开指南\"\n"
        );
        assert_eq!(parse(&content).unwrap(), document());
    }

    #[test]
    fn parse_write_parse_preserves_data_and_canonical_output() {
        let meta = serde_json::to_string(&document().meta).unwrap();
        let original = format!(
            r#"{HEADER}
# 旧版编辑器添加的普通注释
# meta {meta}

## file "chapters/alpha.snbt"

@ {{"file":"chapters/alpha.snbt","entry_id":"alpha:0:description","path":"/extra/0/text","status":"translated"}}
"first line\n第二行 ☃" -> "第一行\nsecond line 🚀"

@ {{"file":"chapters/alpha.snbt","entry_id":"alpha:1:subtitle","path":"$","status":"unchanged"}}
"" -> ""

## file "chapters/zeta.snbt"

@ {{"file":"chapters/zeta.snbt","entry_id":"zeta:0:title","path":"$","status":"review"}}
"contains -> arrow and \"quotes\"" -> "路径 C:\\模组\\任务"
"#
        );
        let first = parse(&original).unwrap();
        let dir = tempdir().unwrap();
        let first_path = dir.path().join("first.cmp");
        let second_path = dir.path().join("second.cmp");

        write(&first_path, &first).unwrap();
        let second = load(&first_path).unwrap();
        assert_eq!(second, first);

        write(&second_path, &second).unwrap();
        assert_eq!(
            fs::read_to_string(first_path).unwrap(),
            fs::read_to_string(second_path).unwrap()
        );
    }

    #[test]
    fn writer_groups_files_in_stable_order_without_reordering_a_file() {
        let mut expected = document();
        expected.records = vec![
            Record {
                file: "chapters/z.snbt".into(),
                entry_id: "z:0:title".into(),
                path: "$".into(),
                source: "Z0".into(),
                target: "零".into(),
                status: "translated".into(),
            },
            Record {
                file: "chapters/a.snbt".into(),
                entry_id: "a:0:title".into(),
                path: "$".into(),
                source: "A0".into(),
                target: "甲".into(),
                status: "translated".into(),
            },
            Record {
                file: "chapters/z.snbt".into(),
                entry_id: "z:1:title".into(),
                path: "$".into(),
                source: "Z1".into(),
                target: "乙".into(),
                status: "review".into(),
            },
        ];
        expected.meta.total_entries = expected.records.len();
        let dir = tempdir().unwrap();
        let path = dir.path().join("ordered.cmp");
        write(&path, &expected).unwrap();

        let parsed = load(&path).unwrap();
        let ids = parsed
            .records
            .iter()
            .map(|record| record.entry_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["a:0:title", "z:0:title", "z:1:title"]);
        let content = fs::read_to_string(path).unwrap();
        assert!(
            content.find("chapters/a.snbt").unwrap() < content.find("chapters/z.snbt").unwrap()
        );
        let location = content.lines().find(|line| line.starts_with("@ ")).unwrap();
        assert_eq!(
            location,
            r#"@ {"file":"chapters/a.snbt","entry_id":"a:0:title","path":"$","status":"translated"}"#
        );
        let meta = content
            .lines()
            .find(|line| line.starts_with("# meta "))
            .unwrap();
        for fields in [
            ["\"version\"", "\"task_id\""],
            ["\"task_id\"", "\"quests_dir\""],
            ["\"quests_dir\"", "\"mode\""],
            ["\"mode\"", "\"source_fingerprint\""],
            ["\"source_fingerprint\"", "\"provider\""],
            ["\"provider\"", "\"base_url\""],
            ["\"base_url\"", "\"model\""],
            ["\"model\"", "\"style\""],
            ["\"style\"", "\"glossary_enabled\""],
            ["\"glossary_enabled\"", "\"glossary_fingerprint\""],
            ["\"glossary_fingerprint\"", "\"total_entries\""],
            ["\"total_entries\"", "\"cache_hits\""],
        ] {
            assert!(meta.find(fields[0]).unwrap() < meta.find(fields[1]).unwrap());
        }
    }

    #[test]
    fn editing_only_the_right_hand_json_string_is_readable() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("review.cmp");
        write(&path, &document()).unwrap();
        let original = fs::read_to_string(&path).unwrap();
        let edited = original.replace(
            r#""Open -> guide\nnow" -> "立即打开指南""#,
            r#""Open -> guide\nnow" -> "引用 \"指南\"，然后打开 C:\\mods""#,
        );
        assert_ne!(edited, original);

        let before = parse(&original).unwrap();
        let after = parse(&edited).unwrap();
        assert_eq!(after.meta, before.meta);
        assert_eq!(after.records[0].file, before.records[0].file);
        assert_eq!(after.records[0].entry_id, before.records[0].entry_id);
        assert_eq!(after.records[0].path, before.records[0].path);
        assert_eq!(after.records[0].source, before.records[0].source);
        assert_eq!(after.records[0].status, before.records[0].status);
        assert_eq!(after.records[0].target, "引用 \"指南\"，然后打开 C:\\mods");
    }

    #[test]
    fn protected_hash_rejects_metadata_and_location_edits() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("protected.cmp");
        write(&path, &document()).unwrap();
        let original = fs::read_to_string(path).unwrap();
        assert!(original.contains("\"protected_hash\""));

        let changed_task =
            original.replace("20260714T120000.000Z-0001", "20260714T120000.000Z-tampered");
        assert!(parse(&changed_task)
            .unwrap_err()
            .user_message
            .contains("受保护内容"));

        let changed_status =
            original.replacen("\"status\":\"translated\"", "\"status\":\"review\"", 1);
        assert!(parse(&changed_status)
            .unwrap_err()
            .user_message
            .contains("受保护内容"));
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
    fn rejects_unknown_status_but_accepts_legacy_fallback() {
        let mut content = String::from(HEADER);
        content.push_str("\n# meta ");
        content.push_str(&serde_json::to_string(&document().meta).unwrap());
        content.push_str("\n@ {\"file\":\"lang/en_us.snbt\",\"entry_id\":\"a\",\"path\":\"$\",\"status\":\"invented\"}\n");
        content.push_str("\"A\" -> \"甲\"\n");
        assert!(parse(&content).is_err());

        let compatible = content.replace("invented", "fallback");
        assert_eq!(parse(&compatible).unwrap().records[0].status, "fallback");
    }

    #[test]
    fn accepts_every_v1_status_and_writer_rejects_unknown_status() {
        for status in [
            "translated",
            "unchanged",
            "review",
            "rate_limited",
            "request_failed",
            "format_guard",
            "fallback",
        ] {
            let mut value = document();
            value.records[0].status = status.into();
            let dir = tempdir().unwrap();
            let path = dir.path().join(format!("{status}.cmp"));
            write(&path, &value).unwrap();
            assert_eq!(load(&path).unwrap().records[0].status, status);
        }

        let mut invalid = document();
        invalid.records[0].status = "invented".into();
        let dir = tempdir().unwrap();
        let error = write(&dir.path().join("invalid.cmp"), &invalid).unwrap_err();
        assert_eq!(error.code, crate::error::ErrorCode::CmpInvalid);
        assert!(error.user_message.contains("invented"));
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
        let content = format!(
            "{HEADER}\n# meta {}\n@ {{\"file\":\"lang/en_us.snbt\",\"entry_id\":\"a\",\"path\":\"$\",\"status\":\"translated\"}}\n\"A\" -> \"甲\"\n",
            serde_json::to_string(&value).unwrap()
        );
        assert!(parse(&content).unwrap().meta.task_id.is_empty());
    }

    #[test]
    fn rejects_missing_required_or_unknown_protected_fields() {
        let mut missing = serde_json::to_value(&document().meta).unwrap();
        missing.as_object_mut().unwrap().remove("provider");
        let missing_content = format!(
            "{HEADER}\n# meta {}\n@ {{\"file\":\"lang/en_us.snbt\",\"entry_id\":\"a\",\"path\":\"$\",\"status\":\"translated\"}}\n\"A\" -> \"甲\"\n",
            serde_json::to_string(&missing).unwrap()
        );
        assert!(parse(&missing_content)
            .unwrap_err()
            .user_message
            .contains("provider"));

        let mut unknown = serde_json::to_value(&document().meta).unwrap();
        unknown
            .as_object_mut()
            .unwrap()
            .insert("future".into(), serde_json::Value::Bool(true));
        let unknown_meta = format!(
            "{HEADER}\n# meta {}\n@ {{\"file\":\"lang/en_us.snbt\",\"entry_id\":\"a\",\"path\":\"$\",\"status\":\"translated\"}}\n\"A\" -> \"甲\"\n",
            serde_json::to_string(&unknown).unwrap()
        );
        assert!(parse(&unknown_meta)
            .unwrap_err()
            .user_message
            .contains("future"));

        let unknown_location = format!(
            "{HEADER}\n# meta {}\n@ {{\"file\":\"lang/en_us.snbt\",\"entry_id\":\"a\",\"path\":\"$\",\"status\":\"translated\",\"future\":true}}\n\"A\" -> \"甲\"\n",
            serde_json::to_string(&document().meta).unwrap()
        );
        assert!(parse(&unknown_location)
            .unwrap_err()
            .user_message
            .contains("future"));

        let missing_location = format!(
            "{HEADER}\n# meta {}\n@ {{\"file\":\"lang/en_us.snbt\",\"entry_id\":\"a\",\"status\":\"translated\"}}\n\"A\" -> \"甲\"\n",
            serde_json::to_string(&document().meta).unwrap()
        );
        assert!(parse(&missing_location)
            .unwrap_err()
            .user_message
            .contains("path"));
    }

    #[test]
    fn rejects_location_that_does_not_match_file_group() {
        let content = format!(
            "{HEADER}\n# meta {}\n## file \"chapters/a.snbt\"\n@ {{\"file\":\"chapters/b.snbt\",\"entry_id\":\"b:0:title\",\"path\":\"$\",\"status\":\"translated\"}}\n\"B\" -> \"乙\"\n",
            serde_json::to_string(&document().meta).unwrap()
        );
        assert!(parse(&content)
            .unwrap_err()
            .user_message
            .contains("## file"));
    }

    #[test]
    fn parse_and_load_errors_keep_cmp_category_and_internal_context() {
        let parse_error = parse("not a cmp").unwrap_err();
        assert_eq!(parse_error.code, crate::error::ErrorCode::CmpInvalid);
        assert!(!parse_error.retryable);
        assert!(!parse_error.task_book_modified);

        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing.cmp");
        let load_error = load(&missing).unwrap_err();
        assert_eq!(load_error.code, crate::error::ErrorCode::CmpInvalid);
        assert!(load_error.user_message.contains("无法读取 CMP 校对文件"));
        assert!(!load_error.internal_message.is_empty());
    }
}
