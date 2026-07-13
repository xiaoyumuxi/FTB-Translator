mod chapters;
mod core;
mod glossary;
mod providers;
mod snbt;
mod storage;

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
    match command.as_str() {
        "scan" => core::scan(&v),
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
        "save-review" => core::save_review(&v),
        _ => Err(format!("未知命令：{command}")),
    }
}

#[tauri::command]
fn start_translation(app: tauri::AppHandle, payload: Value) -> Result<(), String> {
    let dir = data_dir(&app)?;
    let task_app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = core::translate(task_app.clone(), dir, payload).await {
            let _ = task_app.emit("translation-event", json!({"type":"error","message":e}));
        }
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![bridge, start_translation])
        .setup(|app| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title("FTB Translater — 任务书汉化");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running FTB Translater")
}
