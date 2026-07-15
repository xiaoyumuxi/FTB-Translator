use crate::core;
use crate::error::AppError;
pub use crate::protocol::CmpTargetEdit;
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
    pub edits: Vec<CmpTargetEdit>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SaveCmpTargetsResponse {
    pub saved: bool,
    pub entries: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CmpScopeRequest {
    pub cmp_path: String,
    pub quests_dir: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidateCmpRequest {
    pub cmp_path: String,
    pub quests_dir: String,
    #[serde(default)]
    pub edits: Vec<CmpTargetEdit>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ApplyCmpResponse {
    pub report: core::Report,
    pub run_id: i64,
    pub task_id: String,
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

pub fn load_cmp(request: LoadCmpRequest) -> Result<LoadCmpResponse, AppError> {
    let value = core::review_cmp_result(&serde_json::json!({"cmp_path": request.cmp_path}))?;
    serde_json::from_value(value).map_err(|error| invalid_input(error.to_string()))
}

pub fn save_cmp_targets(
    request: SaveCmpTargetsRequest,
) -> Result<SaveCmpTargetsResponse, AppError> {
    let value = core::save_cmp_targets(&request.cmp_path, &request.edits)?;
    serde_json::from_value(value).map_err(|error| invalid_input(error.to_string()))
}

pub fn validate_cmp(request: ValidateCmpRequest) -> Result<core::CmpValidationReport, AppError> {
    core::validate_cmp(
        &serde_json::json!({
            "cmp_path": request.cmp_path,
            "quests_dir": request.quests_dir,
        }),
        &request.edits,
    )
}

pub fn apply_cmp(
    data_dir: &std::path::Path,
    request: CmpScopeRequest,
) -> Result<ApplyCmpResponse, AppError> {
    let value = core::apply_cmp_result(
        data_dir,
        &serde_json::json!({
            "cmp_path": request.cmp_path,
            "quests_dir": request.quests_dir,
        }),
    )?;
    serde_json::from_value(value).map_err(|error| invalid_input(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            "edits": [],
            "source": "tampered"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));

        let valid = serde_json::from_value::<ValidateCmpRequest>(json!({
            "cmp_path": "/tmp/review.cmp",
            "quests_dir": "/tmp/quests",
            "edits": [{"index": 0, "target": "译文"}]
        }));
        assert!(valid.is_ok());
        let error = serde_json::from_value::<ValidateCmpRequest>(json!({
            "cmp_path": "/tmp/review.cmp",
            "quests_dir": "/tmp/quests",
            "edits": [],
            "source": "tampered"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }
}
