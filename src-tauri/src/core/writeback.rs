use super::{
    cache_key, chapters, cmp, entry_source_file, fs, json, load_cache, logging, mode,
    prepare_entry, render_entry, resolve, save_cache, snbt, source_fingerprint,
    validate_cmp_identity, warnings, AppError, AppResult, BTreeMap, EntryKind, HashMap, HashSet,
    History, LangValue, Local, Path, PathBuf, Report, Settings, Value, WalkDir,
};

pub(crate) struct FileOutput {
    pub(crate) path: PathBuf,
    pub(crate) archive_name: String,
    pub(crate) content: String,
}

pub(crate) fn backup(q: &Path, m: &str) -> AppResult<PathBuf> {
    let parent = q.join(".ftb-translater/backups");
    fs::create_dir_all(&parent).map_err(|error| {
        AppError::backup_failed(error.to_string(), format!("{}: {error}", parent.display()))
    })?;
    let timestamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let root = (0..1000)
        .find_map(|attempt| {
            let name = if attempt == 0 {
                timestamp.clone()
            } else {
                format!("{timestamp}-{attempt:03}")
            };
            let candidate = parent.join(name);
            match fs::create_dir(&candidate) {
                Ok(()) => Some(Ok(candidate)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(AppError::backup_failed(
                    error.to_string(),
                    format!("{}: {error}", candidate.display()),
                ))),
            }
        })
        .ok_or_else(|| {
            AppError::backup_failed("无法创建唯一备份目录", parent.display().to_string())
        })??;
    let name = if m == "lang" { "lang" } else { "chapters" };
    for e in WalkDir::new(q.join(name)) {
        let e = e.map_err(|error| {
            AppError::backup_failed(error.to_string(), format!("walk backup tree: {error}"))
        })?;
        let rel = e.path().strip_prefix(q.join(name)).unwrap();
        let dest = root.join(name).join(rel);
        if e.file_type().is_dir() {
            fs::create_dir_all(&dest).map_err(|error| {
                AppError::backup_failed(error.to_string(), format!("{}: {error}", dest.display()))
            })?
        } else {
            fs::copy(e.path(), &dest).map_err(|error| {
                AppError::backup_failed(error.to_string(), format!("{}: {error}", dest.display()))
            })?;
        }
    }
    Ok(root)
}
fn restore_outputs(snapshots: &[(PathBuf, Option<Vec<u8>>)]) -> Vec<String> {
    snapshots
        .iter()
        .rev()
        .filter_map(|(path, original)| {
            let result = match original {
                Some(content) => fs::write(path, content),
                None => match fs::remove_file(path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error),
                },
            };
            result
                .err()
                .map(|error| format!("{}：{error}", path.display()))
        })
        .collect()
}
pub(crate) fn commit_outputs(outputs: &[FileOutput], task_id: &str) -> AppResult<()> {
    let snapshots = outputs
        .iter()
        .map(|output| match fs::read(&output.path) {
            Ok(content) => Ok((output.path.clone(), Some(content))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok((output.path.clone(), None))
            }
            Err(error) => {
                let message = format!("无法读取 {}：{error}", output.path.display());
                Err(AppError::commit_failed(message.clone(), message, false))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (index, output) in outputs.iter().enumerate() {
        if let Err(error) = fs::write(&output.path, &output.content) {
            let rollback_errors = restore_outputs(&snapshots[..=index]);
            logging::error(
                "translation",
                "output_commit_failed",
                "输出文件提交失败并已尝试回滚",
                json!({
                    "task_id":task_id,
                    "path":output.path,
                    "written_before_failure":index,
                    "rollback_succeeded":rollback_errors.is_empty(),
                    "rollback_error_count":rollback_errors.len(),
                    "error":error.to_string()
                }),
            );
            let rollback = if rollback_errors.is_empty() {
                "已恢复此前写入的文件".to_string()
            } else {
                format!("恢复失败：{}", rollback_errors.join("；"))
            };
            let message = format!("无法写入 {}：{error}；{rollback}", output.path.display());
            return Err(AppError::commit_failed(
                message,
                format!(
                    "write failed: {error}; rollback errors: {}",
                    rollback_errors.join("; ")
                ),
                !rollback_errors.is_empty(),
            ));
        }
    }
    Ok(())
}
#[cfg(test)]
pub fn apply_cmp(data_dir: &Path, payload: &Value) -> Result<Value, String> {
    apply_cmp_result(data_dir, payload).map_err(String::from)
}

pub fn apply_cmp_result(data_dir: &Path, payload: &Value) -> AppResult<Value> {
    let cmp_path = PathBuf::from(
        payload["cmp_path"]
            .as_str()
            .ok_or_else(|| AppError::invalid_input("缺少 CMP 文件路径", "cmp_path is missing"))?,
    );
    let operation_id = logging::task_id();
    let document = match cmp::load(&cmp_path) {
        Ok(document) => document,
        Err(error) => {
            logging::warn(
                "translation",
                "cmp_load_failed",
                "CMP 校对文件读取或解析失败",
                json!({"task_id":operation_id,"cmp_path":cmp_path,"error":error}),
            );
            return Err(AppError::cmp_invalid(error.clone(), error));
        }
    };
    let task_id = if document.meta.task_id.trim().is_empty() {
        operation_id
    } else {
        document.meta.task_id.clone()
    };
    logging::info(
        "translation",
        "cmp_apply_started",
        "CMP 校验与写回流程已开始",
        json!({"task_id":task_id,"cmp_path":cmp_path}),
    );
    logging::info(
        "translation",
        "cmp_validation_started",
        "开始重新扫描源文件并校验 CMP",
        json!({"task_id":task_id,"cmp_path":cmp_path}),
    );
    let phase = std::cell::Cell::new("validation");
    let result = apply_cmp_inner(
        Some(data_dir),
        payload,
        cmp_path.clone(),
        document,
        &task_id,
        &phase,
        false,
    );
    if let Err(error) = &result {
        logging::warn(
            "translation",
            "cmp_apply_failed",
            "CMP 校验或写回流程失败",
            json!({"task_id":task_id,"cmp_path":cmp_path,"phase":phase.get(),"error":error}),
        );
    }
    result
}

pub fn validate_cmp(payload: &Value) -> AppResult<Value> {
    let cmp_path = PathBuf::from(
        payload["cmp_path"]
            .as_str()
            .ok_or_else(|| AppError::invalid_input("缺少 CMP 文件路径", "cmp_path is missing"))?,
    );
    let document =
        cmp::load(&cmp_path).map_err(|message| AppError::cmp_invalid(message.clone(), message))?;
    let task_id = if document.meta.task_id.trim().is_empty() {
        logging::task_id()
    } else {
        document.meta.task_id.clone()
    };
    let phase = std::cell::Cell::new("validation");
    apply_cmp_inner(None, payload, cmp_path, document, &task_id, &phase, true)
}

pub(crate) fn apply_cmp_inner(
    data_dir: Option<&Path>,
    payload: &Value,
    cmp_path: PathBuf,
    document: cmp::Document,
    task_id: &str,
    phase: &std::cell::Cell<&str>,
    validate_only: bool,
) -> AppResult<Value> {
    let selected = Path::new(payload["quests_dir"].as_str().ok_or_else(|| {
        AppError::invalid_input(
            "缺少任务书目录",
            "quests_dir is missing from CMP apply payload",
        )
    })?);
    let q = resolve(selected).map_err(|message| {
        AppError::invalid_input(message.clone(), format!("resolve task book: {message}"))
    })?;
    let m = mode(&q).map_err(|message| {
        AppError::invalid_input(message.clone(), format!("detect task book mode: {message}"))
    })?;
    validate_cmp_identity(&document, &q, m)?;

    let mut entries = Vec::new();
    let mut items = Vec::new();
    let mut lang = None;
    let mut chapter_segs = Vec::new();
    if m == "lang" {
        let map = snbt::load(&q.join("lang/en_us.snbt")).map_err(|message| {
            AppError::source_changed(
                message.clone(),
                format!("reload source language: {message}"),
            )
        })?;
        for (entry_index, (key, value)) in map.iter().enumerate() {
            let source = match value {
                LangValue::Text(value) => value.clone(),
                LangValue::Lines(values) => values.join("\n"),
            };
            let (entry, entry_items) = prepare_entry(key.clone(), source, entry_index, None);
            entries.push(entry);
            items.extend(entry_items);
        }
        lang = Some(map);
    } else {
        let mut entry_index = 0;
        for file in chapters::files(&q) {
            for segment in chapters::extract(&file).map_err(|message| {
                AppError::source_changed(message.clone(), format!("reload chapter: {message}"))
            })? {
                let (entry, entry_items) = prepare_entry(
                    segment.cache_id.clone(),
                    segment.source.clone(),
                    entry_index,
                    None,
                );
                entries.push(entry);
                items.extend(entry_items);
                chapter_segs.push(segment);
                entry_index += 1;
            }
        }
    }
    if document.meta.total_entries != entries.len()
        || document.meta.source_fingerprint != source_fingerprint(&entries)
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
    let mut unit_targets = HashMap::new();
    let mut pending_reviews = HashSet::new();
    for record in &document.records {
        let item = expected
            .get(&(record.entry_id.as_str(), record.path.as_str()))
            .ok_or_else(|| {
                let message = format!("CMP 包含未知回填位置：{} {}", record.entry_id, record.path);
                AppError::cmp_invalid(message.clone(), message)
            })?;
        let expected_file = entry_source_file(m, &record.entry_id);
        if record.file != expected_file {
            let message = format!(
                "CMP 文件归属被修改：{} 应属于 {}",
                record.entry_id, expected_file
            );
            return Err(AppError::cmp_invalid(message.clone(), message));
        }
        if record.source != item.source {
            let message = format!(
                "CMP 英文原文被修改：{} {}。只允许修改箭头右侧中文",
                record.entry_id, record.path
            );
            return Err(AppError::cmp_invalid(message.clone(), message));
        }
        if record.target.trim().is_empty() {
            let message = format!("CMP 译文不能为空：{} {}", record.entry_id, record.path);
            return Err(AppError::cmp_invalid(message.clone(), message));
        }
        let problems = warnings(&record.source, &record.target);
        if !problems.is_empty() {
            let message = format!(
                "CMP 译文未通过格式守卫：{} {}（{}）",
                record.entry_id,
                record.path,
                problems.join("；")
            );
            return Err(AppError::format_guard_rejected(message.clone(), message));
        }
        if record.status != "translated" && record.target == record.source {
            pending_reviews.insert(record.entry_id.clone());
        }
        unit_targets.insert(item.id.clone(), record.target.clone());
    }
    if unit_targets.len() != items.len() {
        return Err(AppError::cmp_invalid(
            "CMP 缺少一个或多个回填位置",
            format!("targets={} items={}", unit_targets.len(), items.len()),
        ));
    }

    let mut results = HashMap::new();
    let mut report_warnings = BTreeMap::new();
    let mut details = BTreeMap::new();
    for entry in &entries {
        if let EntryKind::Untouched(reason) = &entry.kind {
            results.insert(entry.id.clone(), entry.source.clone());
            report_warnings.insert(entry.id.clone(), vec![reason.clone()]);
            details.insert(
                entry.id.clone(),
                json!({"source":entry.source,"failed":entry.source}),
            );
            continue;
        }
        let target = render_entry(entry, &unit_targets).map_err(|message| {
            AppError::cmp_invalid(message.clone(), format!("render CMP entry: {message}"))
        })?;
        let problems = warnings(&entry.source, &target);
        if !problems.is_empty() {
            let message = format!(
                "CMP 回填后的完整条目未通过格式守卫：{}（{}）",
                entry.id,
                problems.join("；")
            );
            return Err(AppError::format_guard_rejected(message.clone(), message));
        }
        if pending_reviews.contains(&entry.id) {
            report_warnings.insert(
                entry.id.clone(),
                vec!["机器翻译失败或未通过格式守卫，CMP 仍保留英文原文".into()],
            );
            details.insert(
                entry.id.clone(),
                json!({"source":entry.source,"failed":target}),
            );
        }
        results.insert(entry.id.clone(), target);
    }

    let mut pending_outputs = Vec::new();
    let (source_file, target_file) = if let Some(mut map) = lang {
        for (key, value) in &mut map {
            if let Some(target) = results.get(key) {
                *value = match value {
                    LangValue::Text(_) => LangValue::Text(target.clone()),
                    LangValue::Lines(_) => {
                        LangValue::Lines(target.split('\n').map(str::to_string).collect())
                    }
                };
            }
        }
        let target = q.join("lang/zh_cn.snbt");
        let content = snbt::dump(&map);
        snbt::parse(&content).map_err(|message| {
            AppError::format_guard_rejected(
                message.clone(),
                format!("rendered language SNBT is invalid: {message}"),
            )
        })?;
        pending_outputs.push(FileOutput {
            path: target.clone(),
            archive_name: "lang/zh_cn.snbt".into(),
            content,
        });
        (q.join("lang/en_us.snbt"), target)
    } else {
        let mut by_file: HashMap<PathBuf, Vec<(usize, String)>> = HashMap::new();
        for segment in chapter_segs {
            by_file
                .entry(segment.path)
                .or_default()
                .push((segment.index, results[&segment.cache_id].clone()));
        }
        for (file, replacements) in by_file {
            let (content, _) =
                chapters::render_replacements(&file, &replacements).map_err(|message| {
                    AppError::format_guard_rejected(
                        message.clone(),
                        format!("rendered chapter SNBT is invalid: {message}"),
                    )
                })?;
            pending_outputs.push(FileOutput {
                archive_name: format!("chapters/{}", file.file_name().unwrap().to_string_lossy()),
                path: file,
                content,
            });
        }
        (q.join("chapters"), q.join("chapters"))
    };

    logging::info(
        "translation",
        "cmp_validation_completed",
        "CMP 校对文件及最终输出已通过安全校验",
        json!({
            "task_id":task_id,
            "cmp_path":cmp_path,
            "entries":entries.len(),
            "translation_units":document.records.len(),
            "output_files":pending_outputs.len()
        }),
    );
    if validate_only {
        return Ok(json!({"valid":true}));
    }
    phase.set("backup");
    logging::info(
        "translation",
        "backup_started",
        "开始备份当前任务书",
        json!({"task_id":task_id,"quests_dir":q,"mode":m}),
    );
    let backup = backup(&q, m)?;
    logging::info(
        "translation",
        "backup_completed",
        "当前任务书备份完成",
        json!({"task_id":task_id,"backup_dir":backup}),
    );
    phase.set("commit");
    logging::info(
        "translation",
        "output_commit_started",
        "开始提交全部翻译输出",
        json!({"task_id":task_id,"output_files":pending_outputs.len()}),
    );
    commit_outputs(&pending_outputs, task_id)?;
    logging::info(
        "translation",
        "output_commit_completed",
        "全部翻译输出已提交",
        json!({"task_id":task_id,"output_files":pending_outputs.len()}),
    );
    phase.set("metadata");
    let outputs = pending_outputs
        .into_iter()
        .map(|output| (output.archive_name, output.content, json!({})))
        .collect::<Vec<_>>();
    let settings = Settings {
        provider: document.meta.provider.clone(),
        base_url: document.meta.base_url.clone(),
        model: document.meta.model.clone(),
        style: document.meta.style.clone(),
        glossary_enabled: document.meta.glossary_enabled,
        glossary_fingerprint: document.meta.glossary_fingerprint.clone(),
        ..Settings::default()
    };
    let mut cache = load_cache(&q);
    for entry in &entries {
        let target = results.get(&entry.id).unwrap_or(&entry.source);
        if target != &entry.source {
            cache.insert(cache_key(&entry.source, &settings), target.clone());
        }
    }
    if let Err(error) = save_cache(&q, &cache) {
        logging::warn(
            "translation",
            "cache_update_failed_after_commit",
            "任务书已写入，但翻译缓存更新失败",
            json!({"task_id":task_id,"quests_dir":q,"error":error}),
        );
    }
    let report = Report {
        source_file: source_file.display().to_string(),
        target_file: target_file.display().to_string(),
        backup_dir: backup.display().to_string(),
        total_entries: entries.len(),
        translated_entries: entries.len().saturating_sub(report_warnings.len()),
        cache_hits: document.meta.cache_hits,
        failed_entries: Vec::new(),
        warnings: report_warnings,
        failed_translations: details,
    };
    let report_value = serde_json::to_value(&report)
        .map_err(|error| AppError::commit_failed(error.to_string(), error.to_string(), true))?;
    let data_dir = data_dir.expect("apply mode always provides an application data directory");
    let history_result = match History::new(data_dir) {
        Ok(history) => history
            .insert(&q, m, &settings, &report_value, &outputs)
            .map_err(|error| error.with_task_book_modified(true)),
        Err(message) => Err(AppError::history_save_failed(
            message.clone(),
            message,
            true,
        )),
    };
    let run_id = match history_result {
        Ok(run_id) => run_id,
        Err(error) => {
            logging::warn(
                "translation",
                "history_write_failed_after_commit",
                "任务书已写入，但翻译历史保存失败",
                json!({"task_id":task_id,"quests_dir":q,"error":error}),
            );
            0
        }
    };
    let report_path = q.join(".ftb-translater/report-latest.json");
    let report_result = serde_json::to_vec_pretty(&report_value)
        .map_err(|error| error.to_string())
        .and_then(|bytes| fs::write(&report_path, bytes).map_err(|error| error.to_string()));
    if let Err(error) = report_result {
        logging::warn(
            "translation",
            "report_write_failed_after_commit",
            "任务书已写入，但最新报告保存失败",
            json!({"task_id":task_id,"path":report_path,"error":error}),
        );
    }
    logging::info(
        "translation",
        "cmp_applied",
        "CMP 校对结果已写入任务书",
        json!({"task_id":task_id,"cmp_path":cmp_path,"run_id":run_id,"output_files":outputs.len()}),
    );
    Ok(json!({"report":report,"run_id":run_id,"task_id":task_id}))
}
