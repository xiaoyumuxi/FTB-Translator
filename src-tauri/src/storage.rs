use chrono::Local;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
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
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            has_api_key: false,
            credential_backend: "系统凭证管理器".into(),
            provider: crate::providers::OPENAI_COMPATIBLE.into(),
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-chat".into(),
            style: "自然玩家向简体中文汉化".into(),
            batch_size: "auto".into(),
            concurrency: "auto".into(),
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
}
fn default_provider() -> String {
    crate::providers::OPENAI_COMPATIBLE.into()
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
pub fn load_settings(dir: &Path) -> Settings {
    let mut s = Settings::default();
    if let Ok(raw) = fs::read_to_string(dir.join("settings.json")) {
        if let Ok(c) = serde_json::from_str::<Config>(&raw) {
            s.provider = c.provider;
            s.base_url = c.base_url;
            s.model = c.model;
            s.style = c.style;
            s.batch_size = c.batch_size;
            s.concurrency = c.concurrency;
        }
    }
    if let Ok(e) = entry(&s.provider) {
        if let Ok(k) = e.get_password() {
            s.api_key = k;
            s.has_api_key = true
        }
    }
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
        .unwrap_or(crate::providers::OPENAI_COMPATIBLE);
    crate::providers::normalize(provider)?;
    let key = v["api_key"].as_str().unwrap_or("").trim();
    let e = entry(provider)?;
    if key.is_empty() {
        let _ = e.delete_credential();
    } else {
        e.set_password(key)
            .map_err(|e| format!("无法保存系统凭证：{e}"))?;
    }
    let c = Config {
        provider: provider.into(),
        base_url: parse("base_url")?,
        model: parse("model")?,
        style: parse("style")?,
        batch_size: parse("batch_size")?,
        concurrency: parse("concurrency")?,
    };
    fs::write(
        dir.join("settings.json"),
        serde_json::to_vec_pretty(&c).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({"credential_backend":"系统凭证管理器"}))
}

pub fn provider_credential(provider: &str) -> Result<Value, String> {
    crate::providers::normalize(provider)?;
    let api_key = entry(provider)?.get_password().unwrap_or_default();
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
    ) -> Result<i64, String> {
        let mut c = self.conn()?;
        let tx = c.transaction().map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO translation_runs(quests_dir,pack_name,mode,model,style,total_entries,translated_entries,cache_hits,failed_count,warning_count,created_at)VALUES(?,?,?,?,?,?,?,?,?,?,?)",params![quests.display().to_string(),pack_name(quests),mode,settings.model,settings.style,report["total_entries"].as_i64(),report["translated_entries"].as_i64(),report["cache_hits"].as_i64(),report["failed_entries"].as_array().map_or(0,Vec::len)as i64,report["warnings"].as_object().map_or(0,|x|x.len())as i64,Local::now().to_rfc3339()]).map_err(|e|e.to_string())?;
        let id = tx.last_insert_rowid();
        for (name, content, map) in outputs {
            tx.execute("INSERT INTO translation_files(run_id,filename,mapping,output_content)VALUES(?,?,?,?)",params![id,name,map.to_string(),content]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(id)
    }
    pub fn list(&self) -> Result<Value, String> {
        let c = self.conn()?;
        let mut q=c.prepare("SELECT id,pack_name,quests_dir,mode,model,style,total_entries,translated_entries,cache_hits,failed_count,warning_count,created_at FROM translation_runs ORDER BY created_at DESC,id DESC LIMIT 100").map_err(|e|e.to_string())?;
        let rows=q.query_map([],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"pack_name":r.get::<_,String>(1)?,"quests_dir":r.get::<_,String>(2)?,"mode":r.get::<_,String>(3)?,"model":r.get::<_,String>(4)?,"style":r.get::<_,String>(5)?,"total_entries":r.get::<_,i64>(6)?,"translated_entries":r.get::<_,i64>(7)?,"cache_hits":r.get::<_,i64>(8)?,"failed_count":r.get::<_,i64>(9)?,"warning_count":r.get::<_,i64>(10)?,"created_at":r.get::<_,String>(11)?}))).map_err(|e|e.to_string())?;
        Ok(Value::Array(rows.filter_map(Result::ok).collect()))
    }
    pub fn delete(&self, id: i64) -> Result<(), String> {
        let c = self.conn()?;
        c.execute("DELETE FROM translation_runs WHERE id=?", [id])
            .map_err(|e| e.to_string())?;
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
}
