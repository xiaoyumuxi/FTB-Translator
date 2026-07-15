use crate as crate_root;
use crate::protocol::CmpTargetEdit;
use crate::{
    chapters, cmp,
    error::{AppError, AppResult, ErrorCode},
    glossary, logging, providers, rich_text,
    snbt::{self, LangValue},
    storage::{History, Settings},
};
use chrono::Local;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use walkdir::WalkDir;

#[cfg(test)]
use sha2::Digest;

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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmpValidationReport {
    pub belongs_to_current_task_book: bool,
    pub source_fingerprint_matches: bool,
    /// Entries that can be safely applied, including entries intentionally kept in English.
    pub applicable_entries: usize,
    pub format_guard_failures: usize,
    pub unchanged_entries: usize,
    pub files_to_modify: Vec<String>,
    pub blocking: bool,
    pub blocking_issues: Vec<String>,
}
#[derive(Clone)]
pub(crate) struct Item {
    pub(crate) id: String,
    pub(crate) entry_id: String,
    pub(crate) path: String,
    pub(crate) source: String,
    pub(crate) protected: String,
    pub(crate) tokens: Vec<(String, String)>,
}

pub(crate) struct Entry {
    pub(crate) id: String,
    pub(crate) source: String,
    pub(crate) kind: EntryKind,
}

pub(crate) enum EntryKind {
    Plain(String),
    Untouched(String),
    RichText {
        document: rich_text::Document,
        units: Vec<(String, String)>,
    },
}

impl Entry {
    pub(crate) fn unit_ids(&self) -> Vec<&str> {
        match &self.kind {
            EntryKind::Plain(id) => vec![id],
            EntryKind::Untouched(_) => vec![],
            EntryKind::RichText { units, .. } => units.iter().map(|(_, id)| id.as_str()).collect(),
        }
    }
}

#[path = "core/protection.rs"]
mod protection;
#[path = "core/review.rs"]
mod review;
#[path = "core/scan.rs"]
mod scan;
#[path = "core/translation.rs"]
mod translation;
#[path = "core/writeback.rs"]
mod writeback;

pub fn resolve(selected: &Path) -> Result<PathBuf, String> {
    scan::resolve(selected)
}

pub fn scan(payload: &Value) -> Result<Value, String> {
    scan::scan(payload)
}

pub async fn translate(app: AppHandle, data_dir: PathBuf, payload: Value) -> Result<(), String> {
    translation::translate(app, data_dir, payload).await
}

pub fn apply_cmp_result(data_dir: &Path, payload: &Value) -> AppResult<Value> {
    writeback::apply_cmp_result(data_dir, payload)
}

pub fn validate_cmp(payload: &Value, edits: &[CmpTargetEdit]) -> AppResult<CmpValidationReport> {
    writeback::validate_cmp(payload, edits)
}

pub fn export_cmp(payload: &Value) -> Result<Value, String> {
    review::export_cmp(payload)
}

pub fn review_cmp_result(payload: &Value) -> AppResult<Value> {
    review::review_cmp_result(payload)
}

pub fn save_cmp_targets(path: &str, edits: &[CmpTargetEdit]) -> AppResult<Value> {
    review::save_cmp_targets(path, edits)
}

pub fn open_cmp(payload: &Value) -> Result<Value, String> {
    review::open_cmp(payload)
}

#[cfg(test)]
pub fn apply_cmp(data_dir: &Path, payload: &Value) -> Result<Value, String> {
    writeback::apply_cmp(data_dir, payload)
}

pub(crate) use protection::{prepare_entry, protect, render_entry, restore};
pub(crate) use review::{
    cmp_records, entry_source_file, source_fingerprint, validate_cmp_identity, validate_cmp_source,
};
pub(crate) use scan::{mode, parse_auto};
pub(crate) use translation::{cache_key, load_cache, save_cache, warnings};

#[cfg(test)]
pub(crate) use protection::protect_for_translation;

#[cfg(test)]
pub(crate) use writeback::{apply_cmp_inner, backup, commit_outputs, FileOutput};

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

    fn tree_snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
        WalkDir::new(root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                (
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn cmp_dry_run_collects_counts_and_never_writes() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("config/ftbquests/quests");
        fs::create_dir_all(quests.join("lang")).unwrap();
        fs::write(
            quests.join("lang/en_us.snbt"),
            r#"{ title: "Hello", color: "Use §aGreen", keep: "Keep" }"#,
        )
        .unwrap();
        let mut entries = Vec::new();
        let mut items = Vec::new();
        for (index, (key, value)) in snbt::load(&quests.join("lang/en_us.snbt"))
            .unwrap()
            .into_iter()
            .enumerate()
        {
            let source = match value {
                LangValue::Text(value) => value,
                LangValue::Lines(values) => values.join("\n"),
            };
            let (entry, entry_items) = prepare_entry(key, source, index, None);
            entries.push(entry);
            items.extend(entry_items);
        }
        let cmp_path = directory.path().join("review.cmp");
        cmp::write(
            &cmp_path,
            &cmp::Document {
                meta: cmp::Meta {
                    version: 1,
                    task_id: "dry-run-test".into(),
                    quests_dir: quests.display().to_string(),
                    mode: "lang".into(),
                    source_fingerprint: source_fingerprint(&entries),
                    provider: providers::GOOGLE_WEB.into(),
                    base_url: "https://translate.googleapis.com".into(),
                    model: "google-web".into(),
                    style: "自然中文".into(),
                    glossary_enabled: false,
                    glossary_fingerprint: String::new(),
                    total_entries: entries.len(),
                    cache_hits: 0,
                },
                records: items
                    .iter()
                    .map(|item| cmp::Record {
                        file: "lang/en_us.snbt".into(),
                        entry_id: item.entry_id.clone(),
                        path: item.path.clone(),
                        source: item.source.clone(),
                        target: match item.source.as_str() {
                            "Hello" => "你好".into(),
                            "Use §aGreen" => "使用 §a绿色".into(),
                            source => source.into(),
                        },
                        status: if item.source == "Keep" {
                            "unchanged".into()
                        } else {
                            "translated".into()
                        },
                    })
                    .collect(),
            },
        )
        .unwrap();
        let before = tree_snapshot(directory.path());
        let edits = cmp::load(&cmp_path)
            .unwrap()
            .records
            .iter()
            .enumerate()
            .map(|(index, record)| CmpTargetEdit {
                index,
                target: if record.source == "Use §aGreen" {
                    "错误地删除颜色码".into()
                } else {
                    record.target.clone()
                },
            })
            .collect::<Vec<_>>();

        let report =
            validate_cmp(&json!({"cmp_path":cmp_path,"quests_dir":quests}), &edits).unwrap();

        assert!(report.belongs_to_current_task_book);
        assert!(report.source_fingerprint_matches);
        assert_eq!(report.applicable_entries, 2);
        assert_eq!(report.format_guard_failures, 1);
        assert_eq!(report.unchanged_entries, 1);
        assert_eq!(report.files_to_modify, ["lang/zh_cn.snbt"]);
        assert!(report.blocking);
        assert!(!report.blocking_issues.is_empty());
        assert_eq!(tree_snapshot(directory.path()), before);
        assert!(!quests.join("lang/zh_cn.snbt").exists());
        assert!(!quests.join(".ftb-translater").exists());
    }

    #[test]
    fn cmp_dry_run_returns_a_blocking_report_for_changed_source() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("config/ftbquests/quests");
        fs::create_dir_all(quests.join("lang")).unwrap();
        fs::write(quests.join("lang/en_us.snbt"), r#"{ title: "Hello" }"#).unwrap();
        let cmp_path = directory.path().join("review.cmp");
        write_test_cmp(&cmp_path, &quests, "Hello", "Hello", "你好");
        fs::write(quests.join("lang/en_us.snbt"), r#"{ title: "Changed" }"#).unwrap();

        let report = validate_cmp(&json!({"cmp_path":cmp_path,"quests_dir":quests}), &[]).unwrap();

        assert!(report.belongs_to_current_task_book);
        assert!(!report.source_fingerprint_matches);
        assert!(report.blocking);
        assert_eq!(report.applicable_entries, 0);
        assert_eq!(report.format_guard_failures, 0);
        assert!(report.files_to_modify.is_empty());
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
        let wrong_pack = validate_cmp_source(
            &document,
            &other,
            "lang",
            std::slice::from_ref(&entry),
            &items,
        )
        .unwrap_err();
        assert_eq!(wrong_pack.code, ErrorCode::CmpInvalid);

        document.meta.source_fingerprint = "changed".into();
        let changed = validate_cmp_source(
            &document,
            &quests,
            "lang",
            std::slice::from_ref(&entry),
            &items,
        )
        .unwrap_err();
        assert_eq!(changed.code, ErrorCode::SourceChanged);
        assert!(!changed.retryable);
        assert!(!changed.task_book_modified);

        document.meta.source_fingerprint = source_fingerprint(std::slice::from_ref(&entry));
        document.records[0].source = "Changed".into();
        let changed_english = validate_cmp_source(
            &document,
            &quests,
            "lang",
            std::slice::from_ref(&entry),
            &items,
        )
        .unwrap_err();
        assert_eq!(changed_english.code, ErrorCode::CmpInvalid);
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
    fn typed_cmp_target_edits_preserve_identity_and_cannot_bypass_validation() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("config/ftbquests/quests");
        fs::create_dir_all(quests.join("lang")).unwrap();
        fs::write(
            quests.join("lang/en_us.snbt"),
            r#"{ title: "Use §aGreen" }"#,
        )
        .unwrap();
        let cmp_path = directory.path().join("review.cmp");
        write_test_cmp(
            &cmp_path,
            &quests,
            "Use §aGreen",
            "Use §aGreen",
            "使用 §a绿色",
        );
        let before = cmp::load(&cmp_path).unwrap().records.remove(0);

        save_cmp_targets(
            cmp_path.to_str().unwrap(),
            &[CmpTargetEdit {
                index: 0,
                target: "错误地删除颜色码".into(),
            }],
        )
        .unwrap();

        let after = cmp::load(&cmp_path).unwrap().records.remove(0);
        assert_eq!(after.file, before.file);
        assert_eq!(after.entry_id, before.entry_id);
        assert_eq!(after.path, before.path);
        assert_eq!(after.source, before.source);
        assert_eq!(after.status, before.status);
        assert_eq!(after.target, "错误地删除颜色码");

        let report = validate_cmp(
            &json!({
                "cmp_path": cmp_path,
                "quests_dir": quests,
            }),
            &[],
        )
        .unwrap();
        assert!(report.blocking);
        assert_eq!(report.format_guard_failures, 1);
        assert!(apply_cmp(
            &directory.path().join("app-data"),
            &json!({"cmp_path":cmp_path,"quests_dir":quests}),
        )
        .is_err());
        assert!(!quests.join("lang/zh_cn.snbt").exists());
        assert!(!quests.join(".ftb-translater/backups").exists());
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

        let error = commit_outputs(&outputs, "test-task").unwrap_err();
        assert_eq!(error.code, ErrorCode::CommitFailed);
        assert!(!error.task_book_modified);
        assert!(error.user_message.contains("已恢复此前写入的文件"));
        assert_eq!(fs::read_to_string(first).unwrap(), "original");
    }

    #[test]
    fn backup_failures_are_structured_before_writeback() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("quests");
        fs::create_dir_all(quests.join("lang")).unwrap();
        fs::write(quests.join("lang/en_us.snbt"), "{a: \"A\"}").unwrap();
        fs::write(quests.join(".ftb-translater"), "blocks backup directory").unwrap();

        let error = backup(&quests, "lang").unwrap_err();
        assert_eq!(error.code, ErrorCode::BackupFailed);
        assert!(error.retryable);
        assert!(!error.task_book_modified);
    }

    #[test]
    fn cmp_format_guard_error_is_structured_and_writes_nothing() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("config/ftbquests/quests");
        fs::create_dir_all(quests.join("lang")).unwrap();
        fs::write(
            quests.join("lang/en_us.snbt"),
            r#"{ title: "Use §aGreen" }"#,
        )
        .unwrap();
        let cmp_path = directory.path().join("review.cmp");
        write_test_cmp(&cmp_path, &quests, "Use §aGreen", "Use §aGreen", "使用绿色");
        let document = cmp::load(&cmp_path).unwrap();
        let phase = std::cell::Cell::new("validation");
        let error = apply_cmp_inner(
            Some(&directory.path().join("app-data")),
            &json!({"cmp_path":cmp_path,"quests_dir":quests.display().to_string()}),
            cmp_path,
            document,
            "test-task",
            &phase,
            false,
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::FormatGuardRejected);
        assert!(!error.retryable);
        assert!(!error.task_book_modified);
        assert!(!quests.join("lang/zh_cn.snbt").exists());
        assert!(!quests.join(".ftb-translater/backups").exists());
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
