use crate::core;
use crate::error::AppError;
pub use crate::protocol::CmpTargetEdit;
use crate::task_state::{ActiveTask, TaskState, TaskStateStore};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanRequest {
    pub path: String,
    pub batch_size: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ScanResponse {
    pub quests_dir: String,
    pub pack_name: String,
    pub mode: String,
    pub mode_label: String,
    pub source: String,
    pub entry_count: usize,
    pub file_count: usize,
    pub files: Vec<ScanFileResponse>,
    pub estimated_batches: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ScanFileResponse {
    pub path: String,
    pub entry_count: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TranslateRequest {
    pub quests_dir: String,
    #[serde(default)]
    pub retry_cmp_path: Option<String>,
    pub api_key: String,
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub style: String,
    pub batch_size: String,
    pub concurrency: String,
    pub glossary_enabled: bool,
    pub glossary_path: String,
}

#[derive(Debug, Serialize)]
pub struct TranslateResponse {
    pub accepted: bool,
    pub task_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadCmpRequest {
    pub cmp_path: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LoadCmpResponse {
    pub entries: Vec<CmpEntryResponse>,
    pub task_id: String,
    pub task_state: TaskState,
    pub can_apply: bool,
    pub cmp_revision: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CmpEntryResponse {
    pub index: usize,
    pub entry_id: String,
    pub path: String,
    pub file: String,
    pub source: String,
    pub target: String,
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveCmpTargetsRequest {
    pub cmp_path: String,
    pub expected_revision: String,
    pub edits: Vec<CmpTargetEdit>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SaveCmpTargetsResponse {
    pub saved: bool,
    pub entries: usize,
    pub cmp_revision: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CmpScopeRequest {
    pub cmp_path: String,
    pub quests_dir: String,
    pub cmp_revision: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidateCmpRequest {
    pub cmp_path: String,
    pub quests_dir: String,
    pub cmp_revision: String,
    #[serde(default)]
    pub edits: Vec<CmpTargetEdit>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ApplyCmpResponse {
    pub report: core::Report,
    pub run_id: i64,
    pub task_id: String,
    pub post_commit_warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverTranslationRequest {
    pub quests_dir: String,
}

#[derive(Debug, Serialize)]
pub struct RecoverTranslationResponse {
    pub recovered: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectTaskStateRequest {
    pub quests_dir: String,
}

#[derive(Debug, Serialize)]
pub struct InspectTaskStateResponse {
    pub activities: Vec<ActiveTask>,
}

pub fn invalid_input(error: impl Into<String>) -> AppError {
    let message = error.into();
    AppError::invalid_input(message.clone(), message)
}

pub fn scan(request: ScanRequest) -> Result<ScanResponse, AppError> {
    let value = core::scan(&serde_json::json!({
        "path": request.path,
        "batch_size": request.batch_size,
    }))
    .map_err(invalid_input)?;
    serde_json::from_value(value).map_err(|error| invalid_input(error.to_string()))
}

pub fn load_cmp(
    data_dir: &std::path::Path,
    request: LoadCmpRequest,
) -> Result<LoadCmpResponse, AppError> {
    let (document, cmp_revision) =
        crate::cmp::load_with_revision(std::path::Path::new(&request.cmp_path))?;
    let entries = document
        .records
        .iter()
        .enumerate()
        .map(|(index, record)| CmpEntryResponse {
            index,
            entry_id: record.entry_id.clone(),
            path: record.path.clone(),
            file: record.file.clone(),
            source: record.source.clone(),
            target: record.target.clone(),
            status: record.status.clone(),
        })
        .collect();
    let (_, task_status) = TaskStateStore::new(data_dir)?.register_cmp(&document)?;
    Ok(LoadCmpResponse {
        entries,
        task_id: task_status.task_id,
        task_state: task_status.state,
        can_apply: task_status.can_apply,
        cmp_revision,
    })
}

pub fn save_cmp_targets(
    request: SaveCmpTargetsRequest,
) -> Result<SaveCmpTargetsResponse, AppError> {
    let value = core::save_cmp_targets(
        &request.cmp_path,
        &request.expected_revision,
        &request.edits,
    )?;
    serde_json::from_value(value).map_err(|error| invalid_input(error.to_string()))
}

pub fn validate_cmp(
    data_dir: &std::path::Path,
    request: ValidateCmpRequest,
) -> Result<core::CmpValidationReport, AppError> {
    let report = core::validate_cmp(
        &serde_json::json!({
            "cmp_path": &request.cmp_path,
            "quests_dir": request.quests_dir,
            "cmp_revision": request.cmp_revision,
        }),
        &request.edits,
    )?;
    if !report.blocking {
        let document = crate::cmp::load(std::path::Path::new(&request.cmp_path))?;
        TaskStateStore::new(data_dir)?.register_cmp(&document)?;
    }
    Ok(report)
}

pub fn apply_cmp(
    data_dir: &std::path::Path,
    request: CmpScopeRequest,
) -> Result<ApplyCmpResponse, AppError> {
    let (document, revision) =
        crate::cmp::load_with_revision(std::path::Path::new(&request.cmp_path))?;
    if revision != request.cmp_revision {
        return Err(AppError::cmp_invalid(
            "CMP 已被其他编辑器修改，请重新打开后再应用",
            format!(
                "CMP revision conflict before apply: expected={} actual={revision}",
                request.cmp_revision
            ),
        ));
    }
    let store = TaskStateStore::new(data_dir)?;
    let identity = store.begin_apply(&document)?;
    let value = core::apply_cmp_result(
        data_dir,
        &serde_json::json!({
            "cmp_path": request.cmp_path,
            "quests_dir": request.quests_dir,
            "_task_id": identity.task_id,
            "_cmp_revision": revision,
        }),
    );
    match value {
        Ok(value) => {
            let response: ApplyCmpResponse = serde_json::from_value(value).map_err(|error| {
                AppError::task_state_save_failed(
                    "任务书已写入，但写回结果无法确认。为防止重复写回，任务仍保持正在应用；请勿再次应用",
                    format!("parse committed apply response before marking applied: {error}"),
                    true,
                )
            })?;
            if let Err(error) = store.apply_succeeded(&identity.id) {
                return Err(AppError::task_state_save_failed(
                    "任务书已写入，但无法确认已应用状态。为防止重复写回，任务仍保持正在应用；请勿再次应用并检查应用数据目录",
                    format!("persist applying -> applied: {}", error.internal_message),
                    true,
                ));
            }
            Ok(response)
        }
        Err(error) => {
            if let Err(state_error) = store.apply_failed(&identity.id, error.task_book_modified) {
                return Err(AppError::task_state_save_failed(
                    "写回失败，且无法保存任务恢复状态。为防止重复写回，任务仍保持正在应用；请检查应用数据目录",
                    format!(
                        "apply error: {}; persist applying -> recovery state: {}",
                        error.internal_message, state_error.internal_message
                    ),
                    error.task_book_modified,
                ));
            }
            Err(error)
        }
    }
}

pub fn recover_translation(
    data_dir: &std::path::Path,
    request: RecoverTranslationRequest,
) -> Result<RecoverTranslationResponse, AppError> {
    let store = TaskStateStore::new(data_dir)?;
    let recovered =
        store.recover_interrupted_translation(std::path::Path::new(&request.quests_dir))?;
    Ok(RecoverTranslationResponse { recovered })
}

pub fn inspect_task_state(
    data_dir: &std::path::Path,
    request: InspectTaskStateRequest,
) -> Result<InspectTaskStateResponse, AppError> {
    let activities = TaskStateStore::new(data_dir)?
        .active_operations(std::path::Path::new(&request.quests_dir))?;
    Ok(InspectTaskStateResponse { activities })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cmp, providers};
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn cmp_target_edit_accepts_only_index_and_target() {
        let valid = serde_json::from_value::<CmpTargetEdit>(json!({
            "index": 0,
            "target": "译文"
        }));
        assert!(valid.is_ok());

        for protected in ["source", "path", "entry_id", "file", "status"] {
            let mut value = json!({"index": 0, "target": "译文"});
            value[protected] = json!("tampered");
            let error = serde_json::from_value::<CmpTargetEdit>(value).unwrap_err();
            assert!(
                error.to_string().contains("unknown field"),
                "{protected}: {error}"
            );
        }
    }

    #[test]
    fn cmp_requests_reject_unknown_top_level_fields() {
        let error = serde_json::from_value::<SaveCmpTargetsRequest>(json!({
            "cmp_path": "/tmp/review.cmp",
            "expected_revision": "revision",
            "edits": [],
            "source": "tampered"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));

        let valid = serde_json::from_value::<ValidateCmpRequest>(json!({
            "cmp_path": "/tmp/review.cmp",
            "quests_dir": "/tmp/quests",
            "cmp_revision": "revision",
            "edits": [{"index": 0, "target": "译文"}]
        }));
        assert!(valid.is_ok());
        let error = serde_json::from_value::<ValidateCmpRequest>(json!({
            "cmp_path": "/tmp/review.cmp",
            "quests_dir": "/tmp/quests",
            "cmp_revision": "revision",
            "edits": [],
            "source": "tampered"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn real_load_and_apply_commands_remember_applied_cmp_across_restart() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("quests");
        let data_dir = directory.path().join("app-data");
        fs::create_dir_all(quests.join("lang")).unwrap();
        fs::write(quests.join("lang/en_us.snbt"), r#"{ title: "Hello" }"#).unwrap();
        let (entry, _) = crate::core::prepare_entry("title".into(), "Hello".into(), 0, None);
        let cmp_path = directory.path().join("review.cmp");
        cmp::write(
            &cmp_path,
            &cmp::Document {
                meta: cmp::Meta {
                    version: 1,
                    task_id: "command-apply-task".into(),
                    quests_dir: quests.display().to_string(),
                    mode: "lang".into(),
                    source_fingerprint: crate::core::source_fingerprint(&[entry]),
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
                    source: "Hello".into(),
                    target: "你好".into(),
                    status: "translated".into(),
                }],
            },
        )
        .unwrap();

        let loaded = load_cmp(
            &data_dir,
            LoadCmpRequest {
                cmp_path: cmp_path.display().to_string(),
            },
        )
        .unwrap();
        assert_eq!(loaded.task_state, TaskState::ReviewReady);
        assert!(loaded.can_apply);

        let response = apply_cmp(
            &data_dir,
            CmpScopeRequest {
                cmp_path: cmp_path.display().to_string(),
                quests_dir: quests.display().to_string(),
                cmp_revision: cmp::revision(&cmp_path).unwrap(),
            },
        )
        .unwrap();
        assert_eq!(response.task_id, "command-apply-task");
        let translated = crate::snbt::load(&quests.join("lang/zh_cn.snbt")).unwrap();
        assert_eq!(translated[0].1, crate::snbt::LangValue::Text("你好".into()));

        // Simulate a UI that never received the successful response and clicks again.
        let duplicate = apply_cmp(
            &data_dir,
            CmpScopeRequest {
                cmp_path: cmp_path.display().to_string(),
                quests_dir: quests.display().to_string(),
                cmp_revision: cmp::revision(&cmp_path).unwrap(),
            },
        )
        .unwrap_err();
        assert_eq!(duplicate.code, crate::error::ErrorCode::TaskStateConflict);

        // load_cmp opens a fresh store/connection, matching restart + reimport behavior.
        let reimported = load_cmp(
            &data_dir,
            LoadCmpRequest {
                cmp_path: cmp_path.display().to_string(),
            },
        )
        .unwrap();
        assert_eq!(reimported.task_state, TaskState::Applied);
        assert!(!reimported.can_apply);

        let original = fs::read_to_string(&cmp_path).unwrap();
        fs::write(
            &cmp_path,
            original.replace("command-apply-task", "tampered-apply-task"),
        )
        .unwrap();
        let tampered = load_cmp(
            &data_dir,
            LoadCmpRequest {
                cmp_path: cmp_path.display().to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(tampered.code, crate::error::ErrorCode::CmpInvalid);
    }
}
