use crate::error::{AppError, AppResult};
use chrono::Local;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use zip::{write::SimpleFileOptions, ZipWriter};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub api_key: String,
    pub has_api_key: bool,
    pub credential_backend: String,
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub style: String,
    pub batch_size: String,
    pub concurrency: String,
    pub log_level: String,
    pub glossary_enabled: bool,
    pub glossary_path: String,
    #[serde(default, skip_serializing)]
    pub glossary_fingerprint: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            has_api_key: false,
            credential_backend: "系统凭证管理器".into(),
            provider: crate::providers::GOOGLE_WEB.into(),
            base_url: "https://translate.googleapis.com".into(),
            model: "google-web".into(),
            style: "自然玩家向简体中文汉化".into(),
            batch_size: "auto".into(),
            concurrency: "auto".into(),
            log_level: "info".into(),
            glossary_enabled: false,
            glossary_path: String::new(),
            glossary_fingerprint: String::new(),
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Config {
    #[serde(default = "default_provider")]
    provider: String,
    base_url: String,
    model: String,
    style: String,
    batch_size: String,
    concurrency: String,
    #[serde(default = "default_log_level")]
    log_level: String,
    #[serde(default)]
    glossary_enabled: bool,
    #[serde(default)]
    glossary_path: String,
}
fn default_provider() -> String {
    crate::providers::GOOGLE_WEB.into()
}
fn default_log_level() -> String {
    "info".into()
}
fn entry(provider: &str) -> Result<keyring::Entry, String> {
    let account = if provider == crate::providers::OPENAI_COMPATIBLE {
        "deepseek_api_key".to_string()
    } else {
        format!(
            "{}_api_key",
            provider.replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "")
        )
    };
    keyring::Entry::new("ftb-translater", &account).map_err(|e| e.to_string())
}

fn credential_cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_credential(provider: &str) -> Option<String> {
    credential_cache().lock().ok()?.get(provider).cloned()
}

fn cache_credential(provider: &str, value: Option<&str>) {
    if let Ok(mut cache) = credential_cache().lock() {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            cache.insert(provider.to_string(), value.to_string());
        } else {
            cache.remove(provider);
        }
    }
}

pub fn translation_api_key(provider: &str) -> Result<String, String> {
    crate::providers::normalize(provider)?;
    if let Some(value) = cached_credential(provider) {
        crate::logging::debug(
            "credential",
            "credential_cache_hit",
            "使用当前会话缓存的服务凭证",
            json!({"provider":provider}),
        );
        return Ok(value);
    }
    crate::logging::info(
        "credential",
        "credential_read_started",
        "翻译任务按需读取系统凭证",
        json!({"provider":provider}),
    );
    let value = entry(provider)?
        .get_password()
        .map_err(|_| "没有可用的 API Key，请在设置中查看或修改 API Key 后重试".to_string())?;
    cache_credential(provider, Some(&value));
    crate::logging::info(
        "credential",
        "credential_read_completed",
        "系统凭证已加载到当前会话",
        json!({"provider":provider,"has_value":!value.is_empty()}),
    );
    Ok(value)
}
pub fn load_settings(dir: &Path) -> Settings {
    let glossary_path = crate::glossary::ensure_default(dir)
        .unwrap_or_else(|_| crate::glossary::default_path(dir))
        .display()
        .to_string();
    let mut s = Settings {
        glossary_path,
        ..Settings::default()
    };
    if let Ok(raw) = fs::read_to_string(dir.join("settings.json")) {
        if let Ok(c) = serde_json::from_str::<Config>(&raw) {
            s.provider = c.provider;
            s.base_url = c.base_url;
            s.model = c.model;
            s.style = c.style;
            s.batch_size = c.batch_size;
            s.concurrency = c.concurrency;
            s.log_level = if c.log_level.parse::<crate::logging::Level>().is_ok() {
                c.log_level
            } else {
                default_log_level()
            };
            s.glossary_enabled = c.glossary_enabled;
            if !c.glossary_path.trim().is_empty() {
                s.glossary_path = c.glossary_path;
            }
        }
    }
    crate::logging::debug(
        "settings",
        "settings_loaded",
        "普通设置已加载",
        json!({"provider":s.provider,"log_level":s.log_level,"glossary_enabled":s.glossary_enabled}),
    );
    s
}
pub fn save_settings(dir: &Path, v: &Value) -> Result<Value, String> {
    let parse = |k: &str| -> Result<String, String> {
        let x = v
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("auto")
            .trim()
            .to_string();
        if ["batch_size", "concurrency"].contains(&k)
            && x != "auto"
            && x.parse::<usize>().ok().filter(|n| *n > 0).is_none()
        {
            return Err(format!("{k} 必须是 auto 或正整数"));
        }
        Ok(x)
    };
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let provider = v["provider"]
        .as_str()
        .unwrap_or(crate::providers::GOOGLE_WEB);
    crate::providers::normalize(provider)?;
    let web_provider = !crate::providers::requires_api_key(provider);
    let glossary_enabled = !web_provider && v["glossary_enabled"].as_bool().unwrap_or(false);
    let log_level = v["log_level"].as_str().unwrap_or("info").trim();
    log_level.parse::<crate::logging::Level>()?;
    let glossary_path = match v["glossary_path"].as_str().unwrap_or("").trim() {
        "" => crate::glossary::ensure_default(dir)?,
        path => PathBuf::from(path),
    };
    if glossary_enabled {
        crate::glossary::Loaded::load(&glossary_path)?;
    }
    let c = Config {
        provider: provider.into(),
        base_url: parse("base_url")?,
        model: parse("model")?,
        style: parse("style")?,
        batch_size: if web_provider {
            "auto".into()
        } else {
            parse("batch_size")?
        },
        concurrency: if web_provider {
            "auto".into()
        } else {
            parse("concurrency")?
        },
        log_level: log_level.into(),
        glossary_enabled,
        glossary_path: glossary_path.display().to_string(),
    };

    // Validate every non-sensitive field before changing the system credential.
    // Otherwise a rejected settings form can still replace or delete a working key.
    let config = serde_json::to_vec_pretty(&c).map_err(|e| e.to_string())?;
    fs::write(dir.join("settings.json"), config).map_err(|e| e.to_string())?;
    crate::logging::set_level(log_level)?;
    if v["api_key_changed"].as_bool().unwrap_or(false) {
        let key = v["api_key"].as_str().unwrap_or("").trim();
        let e = entry(provider)?;
        if key.is_empty() {
            let _ = e.delete_credential();
            cache_credential(provider, None);
            crate::logging::info(
                "credential",
                "credential_deleted",
                "用户删除了服务凭证",
                json!({"provider":provider}),
            );
        } else {
            e.set_password(key)
                .map_err(|e| format!("无法保存系统凭证：{e}"))?;
            cache_credential(provider, Some(key));
            crate::logging::info(
                "credential",
                "credential_saved",
                "用户保存了新的服务凭证",
                json!({"provider":provider}),
            );
        }
    }
    crate::logging::info(
        "settings",
        "settings_saved",
        "设置已保存",
        json!({"provider":provider,"log_level":log_level}),
    );
    Ok(
        json!({"credential_backend":"系统凭证管理器","glossary_path":glossary_path,"log_level":log_level}),
    )
}

pub fn provider_credential(provider: &str) -> Result<Value, String> {
    crate::providers::normalize(provider)?;
    let api_key = if let Some(value) = cached_credential(provider) {
        crate::logging::debug(
            "credential",
            "credential_view_cache_hit",
            "从当前会话返回凭证状态",
            json!({"provider":provider}),
        );
        value
    } else {
        crate::logging::info(
            "credential",
            "credential_view_requested",
            "用户明确请求查看系统凭证",
            json!({"provider":provider}),
        );
        let value = entry(provider)?.get_password().unwrap_or_default();
        cache_credential(provider, Some(&value));
        value
    };
    Ok(json!({"api_key":api_key,"has_api_key":!api_key.is_empty()}))
}

pub struct History {
    path: PathBuf,
}
impl History {
    pub fn new(dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let h = Self {
            path: dir.join("history.sqlite3"),
        };
        h.conn()?;
        Ok(h)
    }
    fn conn(&self) -> Result<Connection, String> {
        let c = Connection::open(&self.path).map_err(|e| e.to_string())?;
        c.execute_batch("PRAGMA foreign_keys=ON;CREATE TABLE IF NOT EXISTS translation_runs(id INTEGER PRIMARY KEY,quests_dir TEXT,pack_name TEXT,mode TEXT,model TEXT,style TEXT,total_entries INTEGER,translated_entries INTEGER,cache_hits INTEGER,failed_count INTEGER,warning_count INTEGER,created_at TEXT);CREATE TABLE IF NOT EXISTS translation_files(id INTEGER PRIMARY KEY,run_id INTEGER,filename TEXT,mapping TEXT,output_content TEXT,FOREIGN KEY(run_id) REFERENCES translation_runs(id) ON DELETE CASCADE);").map_err(|e|e.to_string())?;
        Ok(c)
    }
    pub fn insert(
        &self,
        quests: &Path,
        mode: &str,
        settings: &Settings,
        report: &Value,
        outputs: &[(String, String, Value)],
    ) -> AppResult<i64> {
        let history_error =
            |message: String| AppError::history_save_failed(message.clone(), message, false);
        let mut c = self.conn().map_err(history_error)?;
        let tx = c
            .transaction()
            .map_err(|error| history_error(error.to_string()))?;
        tx.execute("INSERT INTO translation_runs(quests_dir,pack_name,mode,model,style,total_entries,translated_entries,cache_hits,failed_count,warning_count,created_at)VALUES(?,?,?,?,?,?,?,?,?,?,?)",params![quests.display().to_string(),pack_name(quests),mode,settings.model,settings.style,report["total_entries"].as_i64(),report["translated_entries"].as_i64(),report["cache_hits"].as_i64(),report["failed_entries"].as_array().map_or(0,Vec::len)as i64,report["warnings"].as_object().map_or(0,|x|x.len())as i64,Local::now().to_rfc3339()]).map_err(|error|history_error(error.to_string()))?;
        let id = tx.last_insert_rowid();
        for (name, content, map) in outputs {
            tx.execute("INSERT INTO translation_files(run_id,filename,mapping,output_content)VALUES(?,?,?,?)",params![id,name,map.to_string(),content]).map_err(|error|history_error(error.to_string()))?;
        }
        tx.commit()
            .map_err(|error| history_error(error.to_string()))?;
        crate::logging::info(
            "history",
            "history_inserted",
            "翻译历史已保存",
            json!({"run_id":id,"mode":mode,"pack_name":pack_name(quests)}),
        );
        Ok(id)
    }
    pub fn list(&self) -> Result<Value, String> {
        let c = self.conn()?;
        let mut q=c.prepare("SELECT id,pack_name,quests_dir,mode,model,style,total_entries,translated_entries,cache_hits,failed_count,warning_count,created_at FROM translation_runs ORDER BY created_at DESC,id DESC LIMIT 100").map_err(|e|e.to_string())?;
        let rows=q.query_map([],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"pack_name":r.get::<_,String>(1)?,"quests_dir":r.get::<_,String>(2)?,"mode":r.get::<_,String>(3)?,"model":r.get::<_,String>(4)?,"style":r.get::<_,String>(5)?,"total_entries":r.get::<_,i64>(6)?,"translated_entries":r.get::<_,i64>(7)?,"cache_hits":r.get::<_,i64>(8)?,"failed_count":r.get::<_,i64>(9)?,"warning_count":r.get::<_,i64>(10)?,"created_at":r.get::<_,String>(11)?}))).map_err(|e|e.to_string())?;
        let rows = rows.filter_map(Result::ok).collect::<Vec<_>>();
        crate::logging::debug(
            "history",
            "history_listed",
            "翻译历史列表已读取",
            json!({"count":rows.len()}),
        );
        Ok(Value::Array(rows))
    }
    pub fn delete(&self, id: i64) -> Result<(), String> {
        let c = self.conn()?;
        c.execute("DELETE FROM translation_runs WHERE id=?", [id])
            .map_err(|e| e.to_string())?;
        crate::logging::info(
            "history",
            "history_deleted",
            "翻译历史已删除",
            json!({"run_id":id}),
        );
        Ok(())
    }
    pub fn export(&self, id: i64, dest: &Path) -> Result<(), String> {
        let c = self.conn()?;
        let mode: String = c
            .query_row("SELECT mode FROM translation_runs WHERE id=?", [id], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        let mut q = c
            .prepare("SELECT filename,output_content FROM translation_files WHERE run_id=?")
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let file = fs::File::create(dest).map_err(|e| e.to_string())?;
        let mut zip = ZipWriter::new(file);
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("manifest.json", opts)
            .map_err(|e| e.to_string())?;
        zip.write_all(json!({"run_id":id,"mode":mode}).to_string().as_bytes())
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (name, content) = row.map_err(|e| e.to_string())?;
            if name.contains("..") || name.starts_with('/') {
                return Err("历史文件路径不安全".into());
            }
            zip.start_file(name, opts).map_err(|e| e.to_string())?;
            zip.write_all(content.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
        crate::logging::info(
            "history",
            "history_exported",
            "翻译历史已导出",
            json!({"run_id":id,"destination":dest}),
        );
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn defaults_to_google_web_with_optional_glossary_disabled() {
        let settings = Settings::default();
        assert_eq!(settings.provider, crate::providers::GOOGLE_WEB);
        assert_eq!(settings.base_url, "https://translate.googleapis.com");
        assert_eq!(settings.model, "google-web");
        assert!(!settings.glossary_enabled);

        let old_config = r#"{
            "provider":"openai_compatible",
            "base_url":"https://api.deepseek.com",
            "model":"deepseek-chat",
            "style":"自然中文",
            "batch_size":"auto",
            "concurrency":"auto"
        }"#;
        let config: Config = serde_json::from_str(old_config).unwrap();
        assert!(!config.glossary_enabled);
    }

    #[test]
    fn settings_expose_an_editable_default_glossary_path() {
        let d = tempdir().unwrap();
        let settings = load_settings(d.path());
        let path = PathBuf::from(&settings.glossary_path);
        assert!(path.is_file());
        assert_eq!(path.file_name().unwrap(), crate::glossary::DEFAULT_FILENAME);
        fs::write(
            &path,
            r#"{"version":3,"entries":[{"source":"Custom","target":"自定义"}]}"#,
        )
        .unwrap();
        load_settings(d.path());
        assert!(fs::read_to_string(path).unwrap().contains("Custom"));
    }

    #[test]
    fn web_provider_persists_only_automatic_settings() {
        let d = tempdir().unwrap();
        save_settings(
            d.path(),
            &json!({
                "provider":crate::providers::GOOGLE_WEB,
                "base_url":"https://translate.googleapis.com",
                "model":"google-web",
                "style":"ignored",
                "batch_size":"99",
                "concurrency":"8",
                "glossary_enabled":true,
                "glossary_path":"",
                "api_key_changed":false
            }),
        )
        .unwrap();
        let settings = load_settings(d.path());
        assert!(!settings.glossary_enabled);
        assert_eq!(settings.batch_size, "auto");
        assert_eq!(settings.concurrency, "auto");
    }

    #[test]
    fn invalid_settings_do_not_replace_the_saved_configuration() {
        let d = tempdir().unwrap();
        let invalid_glossary = d.path().join("invalid.json");
        fs::write(&invalid_glossary, "not json").unwrap();

        let error = save_settings(
            d.path(),
            &json!({
                "provider":crate::providers::DEEPL,
                "base_url":"https://api-free.deepl.com",
                "model":"deepl",
                "style":"自然中文",
                "batch_size":"auto",
                "concurrency":"auto",
                "glossary_enabled":true,
                "glossary_path":invalid_glossary,
                "api_key_changed":false
            }),
        )
        .unwrap_err();

        assert!(error.contains("词表"));
        assert!(!d.path().join("settings.json").exists());
    }

    #[test]
    fn ordinary_settings_save_reuses_session_key_without_keyring_write() {
        let d = tempdir().unwrap();
        let provider = crate::providers::OPENAI_COMPATIBLE;
        cache_credential(provider, Some("session-only-key"));
        save_settings(
            d.path(),
            &json!({
                "provider":provider,
                "base_url":"https://api.deepseek.com",
                "model":"deepseek-chat",
                "style":"自然中文",
                "batch_size":"auto",
                "concurrency":"auto",
                "glossary_enabled":false,
                "api_key":"",
                "api_key_changed":false
            }),
        )
        .unwrap();
        assert_eq!(translation_api_key(provider).unwrap(), "session-only-key");
        cache_credential(provider, None);
    }

    #[test]
    fn history_roundtrip_and_export() {
        let d = tempdir().unwrap();
        let history = History::new(d.path()).unwrap();
        let report = json!({
            "total_entries": 2,
            "translated_entries": 2,
            "cache_hits": 1,
            "failed_entries": [],
            "warnings": {}
        });
        let id = history
            .insert(
                Path::new("/packs/demo/config/ftbquests/quests"),
                "lang",
                &Settings::default(),
                &report,
                &[("lang/zh_cn.snbt".into(), "{\"a\":\"甲\"}".into(), json!({}))],
            )
            .unwrap();
        assert_eq!(history.list().unwrap().as_array().unwrap().len(), 1);
        let archive = d.path().join("translation.zip");
        history.export(id, &archive).unwrap();
        assert!(archive.is_file());
        history.delete(id).unwrap();
        assert!(history.list().unwrap().as_array().unwrap().is_empty());
    }

    #[test]
    fn history_insert_failure_is_structured_without_assuming_writeback() {
        let directory = tempdir().unwrap();
        let history = History {
            path: directory.path().to_path_buf(),
        };
        let error = history
            .insert(
                Path::new("/packs/demo/config/ftbquests/quests"),
                "lang",
                &Settings::default(),
                &json!({
                    "total_entries": 0,
                    "translated_entries": 0,
                    "cache_hits": 0,
                    "failed_entries": [],
                    "warnings": {}
                }),
                &[],
            )
            .unwrap_err();
        assert_eq!(error.code, crate::error::ErrorCode::HistorySaveFailed);
        assert!(error.retryable);
        assert!(!error.task_book_modified);
        assert!(!error.internal_message.is_empty());
    }
}
