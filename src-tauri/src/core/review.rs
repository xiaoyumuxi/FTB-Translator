use super::{
    chapters, cmp, fs, json, logging, rich_text, AppError, AppResult, BTreeMap, CmpTargetEdit,
    Entry, EntryKind, HashMap, HashSet, Item, Path, PathBuf, Sha256, Value,
};
use sha2::Digest;
use std::process::Command;

pub(crate) fn source_fingerprint(entries: &[Entry]) -> String {
    let mut hash = Sha256::new();
    for entry in entries {
        hash.update(entry.id.as_bytes());
        hash.update([0]);
        hash.update(entry.source.as_bytes());
        hash.update([0xff]);
    }
    hex::encode(hash.finalize())
}

pub(crate) fn current_source_fingerprint(
    quests_dir: &Path,
    mode: &str,
    entries: &[Entry],
) -> AppResult<String> {
    let files = if mode == "lang" {
        vec![quests_dir.join("lang/en_us.snbt")]
    } else {
        chapters::files(quests_dir)
    };
    let mut hash = Sha256::new();
    hash.update(b"ftb-translator-source-v2\0");
    hash.update(if mode == "lang" {
        b"lang-parser-v1".as_slice()
    } else {
        b"chapter-token-walker-v1".as_slice()
    });
    hash.update([0]);
    hash.update(source_fingerprint(entries).as_bytes());
    for path in files {
        let relative = path.strip_prefix(quests_dir).map_err(|error| {
            AppError::source_changed(
                "无法建立任务书源文件指纹",
                format!(
                    "source path {} is outside task book: {error}",
                    path.display()
                ),
            )
        })?;
        let bytes = fs::read(&path).map_err(|error| {
            AppError::source_changed(
                format!("无法读取任务书源文件 {}：{error}", path.display()),
                error.to_string(),
            )
        })?;
        let relative = relative.to_string_lossy();
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Ok(format!("v2:{}", hex::encode(hash.finalize())))
}

pub(crate) fn source_fingerprint_matches(
    recorded: &str,
    quests_dir: &Path,
    mode: &str,
    entries: &[Entry],
) -> AppResult<bool> {
    if recorded.starts_with("v2:") {
        Ok(recorded == current_source_fingerprint(quests_dir, mode, entries)?)
    } else {
        // CMP v1 files created before source-v2 hashed only extracted IDs and English.
        Ok(recorded == source_fingerprint(entries))
    }
}

pub(crate) fn validate_cmp_identity(
    document: &cmp::Document,
    quests_dir: &Path,
    mode: &str,
) -> AppResult<()> {
    let current = quests_dir.canonicalize().map_err(|error| {
        AppError::cmp_invalid(error.to_string(), format!("quests directory: {error}"))
    })?;
    let recorded = PathBuf::from(&document.meta.quests_dir)
        .canonicalize()
        .map_err(|error| {
            AppError::cmp_invalid(
                format!("CMP 中的任务书目录不可用：{error}"),
                error.to_string(),
            )
        })?;
    if recorded != current {
        return Err(AppError::cmp_invalid(
            "CMP 不属于当前扫描的任务书目录",
            format!(
                "recorded={} current={}",
                recorded.display(),
                current.display()
            ),
        ));
    }
    if document.meta.mode != mode {
        return Err(AppError::cmp_invalid(
            "CMP 的任务书模式与当前目录不一致",
            format!("recorded={} current={mode}", document.meta.mode),
        ));
    }
    Ok(())
}

pub(crate) fn validate_cmp_source(
    document: &cmp::Document,
    quests_dir: &Path,
    mode: &str,
    entries: &[Entry],
    items: &[Item],
) -> AppResult<()> {
    validate_cmp_identity(document, quests_dir, mode)?;
    if document.meta.total_entries != entries.len()
        || !source_fingerprint_matches(
            &document.meta.source_fingerprint,
            quests_dir,
            mode,
            entries,
        )?
    {
        return Err(AppError::source_changed(
            "任务书内容在 CMP 生成后发生了变化，请重新扫描并翻译",
            "CMP entry count or source fingerprint differs from the current task book",
        ));
    }
    if document.records.len() != items.len() {
        return Err(AppError::cmp_invalid(
            "CMP 翻译条目数量与当前任务书不一致",
            format!(
                "cmp_records={} current_items={}",
                document.records.len(),
                items.len()
            ),
        ));
    }
    let expected = items
        .iter()
        .map(|item| ((item.entry_id.as_str(), item.path.as_str()), item))
        .collect::<HashMap<_, _>>();
    for record in &document.records {
        let item = expected
            .get(&(record.entry_id.as_str(), record.path.as_str()))
            .ok_or_else(|| {
                let message = format!("CMP 包含未知回填位置：{} {}", record.entry_id, record.path);
                AppError::cmp_invalid(message.clone(), message)
            })?;
        if record.file != entry_source_file(mode, &record.entry_id) {
            let message = format!("CMP 文件归属被修改：{}", record.entry_id);
            return Err(AppError::cmp_invalid(message.clone(), message));
        }
        if record.source != item.source {
            let message = format!(
                "CMP 英文原文被修改：{} {}。只允许修改箭头右侧中文",
                record.entry_id, record.path
            );
            return Err(AppError::cmp_invalid(message.clone(), message));
        }
    }
    Ok(())
}

pub(crate) fn entry_source_file(mode: &str, entry_id: &str) -> String {
    if mode == "lang" {
        "lang/en_us.snbt".into()
    } else {
        format!(
            "chapters/{}",
            entry_id.split_once(':').map_or(entry_id, |(file, _)| file)
        )
    }
}

pub(crate) fn cmp_records(
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

pub fn save_cmp_targets(
    path: &str,
    expected_revision: &str,
    edits: &[CmpTargetEdit],
) -> AppResult<Value> {
    let path = Path::new(path);
    let (mut document, revision) = cmp::load_with_revision(path)?;
    if revision != expected_revision {
        return Err(AppError::cmp_invalid(
            "CMP 已被其他编辑器修改，请重新打开后再保存",
            format!("CMP revision conflict: expected={expected_revision} actual={revision}"),
        ));
    }
    if edits.len() != document.records.len() {
        return Err(AppError::cmp_invalid(
            "CMP 校对表格条目数已变化，请重新打开校对表格",
            format!(
                "submitted edits={} cmp records={}",
                edits.len(),
                document.records.len()
            ),
        ));
    }
    for (expected_index, edit) in edits.iter().enumerate() {
        if edit.index != expected_index {
            return Err(AppError::cmp_invalid(
                "CMP 校对表格内容已过期或不属于当前文件，请重新打开",
                format!(
                    "submitted index={} expected index={expected_index}",
                    edit.index
                ),
            ));
        }
        if edit.target.trim().is_empty() {
            let message = format!("第 {} 条译文不能为空", expected_index + 1);
            return Err(AppError::cmp_invalid(message.clone(), message));
        }
        let record = &mut document.records[edit.index];
        let target_changed = record.target != edit.target;
        record.target.clone_from(&edit.target);
        if target_changed
            && matches!(
                record.status.as_str(),
                "rate_limited" | "request_failed" | "format_guard" | "unchanged" | "fallback"
            )
            && record.target != record.source
        {
            record.status = "review".into();
        }
    }
    let current_revision = cmp::revision(path)?;
    if current_revision != revision {
        return Err(AppError::cmp_invalid(
            "CMP 在保存期间被其他编辑器修改，请重新打开后再试",
            format!("CMP changed during save: loaded={revision} current={current_revision}"),
        ));
    }
    cmp::write(path, &document)?;
    let revision = cmp::revision(path)?;
    logging::info(
        "translation",
        "cmp_edits_saved",
        "CMP 校对表格修改已保存",
        json!({"task_id":document.meta.task_id,"cmp_path":path,"entries":edits.len()}),
    );
    Ok(json!({"saved":true,"entries":edits.len(),"cmp_revision":revision}))
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
