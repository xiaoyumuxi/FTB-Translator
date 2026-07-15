mod chapters;
mod cmp;
mod commands;
mod core;
mod error;
mod glossary;
mod logging;
mod protocol;
mod providers;
mod rich_text;
mod snbt;
mod storage;
mod task_state;

use error::AppError;
use serde_json::{json, Value};
use std::path::PathBuf;
use storage::History;
use tauri::{Emitter, Manager};

fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

#[tauri::command]
fn bridge(app: tauri::AppHandle, command: String, payload: Option<Value>) -> Result<Value, String> {
    let v = payload.unwrap_or_else(|| json!({}));
    let dir = data_dir(&app)?;
    logging::trace(
        "command",
        "command_started",
        "应用命令已调用",
        json!({"command":command}),
    );
    let result = match command.as_str() {
        "settings" => serde_json::to_value(storage::load_settings(&dir)).map_err(|e| e.to_string()),
        "save-settings" => storage::save_settings(&dir, &v),
        "default-glossary" => {
            let path = glossary::ensure_default(&dir)?;
            Ok(json!({"path":path}))
        }
        "provider-credential" => {
            storage::provider_credential(v["provider"].as_str().ok_or("缺少翻译提供商")?)
        }
        "history-list" => History::new(&dir)?.list(),
        "history-delete" => {
            History::new(&dir)?.delete(v["run_id"].as_i64().ok_or("缺少历史编号")?)?;
            Ok(json!({"deleted":true}))
        }
        "history-export" => {
            History::new(&dir)?.export(
                v["run_id"].as_i64().ok_or("缺少历史编号")?,
                std::path::Path::new(v["path"].as_str().ok_or("缺少导出路径")?),
            )?;
            Ok(json!({"path":v["path"]}))
        }
        "frontend-log" => {
            logging::frontend(
                v["level"].as_str().unwrap_or("info"),
                v["event"].as_str().unwrap_or("frontend_event"),
                v["message"].as_str().unwrap_or("前端事件"),
                v.get("context").cloned().unwrap_or_else(|| json!({})),
            )?;
            Ok(json!({"written":true}))
        }
        "logs-info" => Ok(json!({
            "directory":logging::directory()?,
            "backend":"backend.log",
            "frontend":"frontend.log"
        })),
        "logs-open" => {
            logging::open_directory()?;
            Ok(json!({"opened":true}))
        }
        "logs-export" => {
            let path = std::path::Path::new(v["path"].as_str().ok_or("缺少日志导出路径")?);
            logging::export(path)?;
            logging::info(
                "diagnostics",
                "logs_exported",
                "诊断日志已导出",
                json!({"path":path}),
            );
            Ok(json!({"path":path}))
        }
        "cmp-export" => core::export_cmp(&v),
        "cmp-open" => core::open_cmp(&v),
        _ => Err(format!("未知命令：{command}")),
    };
    if let Err(error) = &result {
        logging::warn(
            "command",
            "command_failed",
            "应用命令执行失败",
            json!({"command":command,"error":error}),
        );
    }
    result
}

#[tauri::command]
fn scan(request: commands::ScanRequest) -> Result<commands::ScanResponse, AppError> {
    commands::scan(request)
}

#[tauri::command]
fn translate(
    app: tauri::AppHandle,
    request: commands::TranslateRequest,
) -> Result<commands::TranslateResponse, AppError> {
    let dir = data_dir(&app).map_err(commands::invalid_input)?;
    let mut payload = serde_json::to_value(request)
        .map_err(|error| commands::invalid_input(error.to_string()))?;
    let task_app = app.clone();
    let store = task_state::TaskStateStore::new(&dir)?;
    let identity = if let Some(path) = payload["retry_cmp_path"]
        .as_str()
        .filter(|path| !path.trim().is_empty())
    {
        let document = cmp::load(std::path::Path::new(path))
            .map_err(|message| AppError::cmp_invalid(message.clone(), message))?;
        store.reserve_retry_translation(&document)?
    } else {
        let quests_dir = std::path::Path::new(
            payload["quests_dir"]
                .as_str()
                .ok_or_else(|| commands::invalid_input("缺少任务书目录"))?,
        );
        store.reserve_new_translation(quests_dir, &logging::task_id())?
    };
    let task_id = identity.task_id.clone();
    payload["_task_id"] = json!(task_id);
    let response_task_id = task_id.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = core::translate(task_app.clone(), dir, payload).await {
            if let Err(state_error) = store.translation_failed(&identity.id) {
                logging::error(
                    "translation",
                    "task_failure_state_save_failed",
                    "翻译失败，且任务状态无法更新",
                    json!({"task_id":task_id,"error":state_error}),
                );
            }
            logging::error(
                "translation",
                "task_failed",
                "翻译任务失败",
                json!({"task_id":task_id,"error":e}),
            );
            let _ = task_app.emit(
                "translation-event",
                json!({"type":"error","task_id":task_id,"message":e}),
            );
        }
    });
    Ok(commands::TranslateResponse {
        accepted: true,
        task_id: response_task_id,
    })
}

#[tauri::command]
fn load_cmp(
    app: tauri::AppHandle,
    request: commands::LoadCmpRequest,
) -> Result<commands::LoadCmpResponse, AppError> {
    let dir = data_dir(&app).map_err(commands::invalid_input)?;
    commands::load_cmp(&dir, request)
}

#[tauri::command]
fn save_cmp_targets(
    request: commands::SaveCmpTargetsRequest,
) -> Result<commands::SaveCmpTargetsResponse, AppError> {
    commands::save_cmp_targets(request)
}

#[tauri::command]
fn validate_cmp(
    app: tauri::AppHandle,
    request: commands::ValidateCmpRequest,
) -> Result<core::CmpValidationReport, AppError> {
    let dir = data_dir(&app).map_err(commands::invalid_input)?;
    commands::validate_cmp(&dir, request)
}

#[tauri::command]
fn apply_cmp(
    app: tauri::AppHandle,
    request: commands::CmpScopeRequest,
) -> Result<commands::ApplyCmpResponse, AppError> {
    let dir = data_dir(&app).map_err(commands::invalid_input)?;
    commands::apply_cmp(&dir, request)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            bridge,
            scan,
            translate,
            load_cmp,
            save_cmp_targets,
            validate_cmp,
            apply_cmp
        ])
        .setup(|app| {
            let dir = data_dir(app.handle()).map_err(std::io::Error::other)?;
            let settings = storage::load_settings(&dir);
            match logging::init(&settings.log_level) {
                Ok(_) => {
                    logging::info(
                        "app",
                        "application_started",
                        "应用程序已启动",
                        json!({"version":env!("CARGO_PKG_VERSION")}),
                    );
                    logging::debug(
                        "settings",
                        "startup_settings_loaded",
                        "启动设置已加载",
                        json!({
                            "provider":settings.provider,
                            "log_level":settings.log_level,
                            "glossary_enabled":settings.glossary_enabled
                        }),
                    );
                }
                Err(error) => eprintln!("{error}"),
            }
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title("FTB Translater — 任务书汉化");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running FTB Translater")
}
