use super::{
    cache_key, chapters, cmp, entry_source_file, fs, json, load_cache, logging, mode,
    prepare_entry, render_entry, resolve, save_cache, snbt, source_fingerprint,
    validate_cmp_identity, warnings, AppError, AppResult, BTreeMap, CmpTargetEdit,
    CmpValidationReport, Entry, EntryKind, HashMap, HashSet, History, Item, LangValue, Local, Path,
    PathBuf, Report, Settings, Value, WalkDir,
};
use std::{
    fs::OpenOptions,
    io::{self, Write},
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
pub(crate) struct WritebackOptions {
    #[cfg(test)]
    pub(crate) backup_fault: BackupFault,
    #[cfg(test)]
    pub(crate) commit_fault: CommitFault,
    #[cfg(test)]
    pub(crate) auxiliary_fault: AuxiliaryFault,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum BackupFault {
    #[default]
    None,
    ParentCreate,
    SnapshotCreate,
    CopyFile(usize),
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum CommitFault {
    #[default]
    None,
    TempCreate(usize),
    TempPrepare(usize),
    Replace(usize),
    ReplaceAndRollback {
        replace_index: usize,
        rollback_index: usize,
    },
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AuxiliaryFault {
    #[default]
    None,
    History,
    Report,
}

impl WritebackOptions {
    fn fail_backup_parent(&self) -> bool {
        #[cfg(test)]
        return self.backup_fault == BackupFault::ParentCreate;
        #[cfg(not(test))]
        return false;
    }

    fn fail_backup_snapshot(&self) -> bool {
        #[cfg(test)]
        return self.backup_fault == BackupFault::SnapshotCreate;
        #[cfg(not(test))]
        return false;
    }

    fn fail_backup_copy(&self, index: usize) -> bool {
        #[cfg(test)]
        return self.backup_fault == BackupFault::CopyFile(index);
        #[cfg(not(test))]
        {
            let _ = index;
            false
        }
    }

    fn fail_temp_create(&self, index: usize) -> bool {
        #[cfg(test)]
        return self.commit_fault == CommitFault::TempCreate(index);
        #[cfg(not(test))]
        {
            let _ = index;
            false
        }
    }

    fn fail_replace(&self, index: usize) -> bool {
        #[cfg(test)]
        return self.commit_fault == CommitFault::Replace(index)
            || matches!(
                self.commit_fault,
                CommitFault::ReplaceAndRollback { replace_index, .. } if replace_index == index
            );
        #[cfg(not(test))]
        {
            let _ = index;
            false
        }
    }

    fn fail_temp_prepare(&self, index: usize) -> bool {
        #[cfg(test)]
        return self.commit_fault == CommitFault::TempPrepare(index);
        #[cfg(not(test))]
        {
            let _ = index;
            false
        }
    }

    fn fail_rollback(&self, index: usize) -> bool {
        #[cfg(test)]
        return matches!(
            self.commit_fault,
            CommitFault::ReplaceAndRollback { rollback_index, .. } if rollback_index == index
        );
        #[cfg(not(test))]
        {
            let _ = index;
            false
        }
    }

    fn fail_history(&self) -> bool {
        #[cfg(test)]
        return self.auxiliary_fault == AuxiliaryFault::History;
        #[cfg(not(test))]
        return false;
    }

    fn fail_report(&self) -> bool {
        #[cfg(test)]
        return self.auxiliary_fault == AuxiliaryFault::Report;
        #[cfg(not(test))]
        return false;
    }
}

fn injected_error(step: &str) -> io::Error {
    io::Error::other(format!("test fault injected at {step}"))
}

pub(crate) struct FileOutput {
    pub(crate) path: PathBuf,
    pub(crate) archive_name: String,
    pub(crate) content: String,
}

pub(crate) fn backup_with_options(
    q: &Path,
    m: &str,
    options: &WritebackOptions,
) -> AppResult<PathBuf> {
    let parent = q.join(".ftb-translater/backups");
    let parent_result = if options.fail_backup_parent() {
        Err(injected_error("backup parent create"))
    } else {
        fs::create_dir_all(&parent)
    };
    parent_result.map_err(|error| {
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
            let result = if options.fail_backup_snapshot() {
                Err(injected_error("backup snapshot create"))
            } else {
                fs::create_dir(&candidate)
            };
            match result {
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
    let backup_result = (|| {
        let mut file_index = 0;
        for entry in WalkDir::new(q.join(name)) {
            let entry = entry.map_err(|error| {
                AppError::backup_failed(error.to_string(), format!("walk backup tree: {error}"))
            })?;
            let rel = entry.path().strip_prefix(q.join(name)).unwrap();
            let dest = root.join(name).join(rel);
            if entry.file_type().is_dir() {
                fs::create_dir_all(&dest).map_err(|error| {
                    AppError::backup_failed(
                        error.to_string(),
                        format!("{}: {error}", dest.display()),
                    )
                })?;
            } else {
                let copy_result = if options.fail_backup_copy(file_index) {
                    Err(injected_error("backup file copy"))
                } else {
                    fs::copy(entry.path(), &dest)
                };
                copy_result.map_err(|error| {
                    AppError::backup_failed(
                        error.to_string(),
                        format!("{}: {error}", dest.display()),
                    )
                })?;
                file_index += 1;
            }
        }
        Ok(())
    })();
    if let Err(error) = backup_result {
        let _ = fs::remove_dir_all(&root);
        return Err(error);
    }
    Ok(root)
}

struct StagedOutput {
    index: usize,
    target: PathBuf,
    temporary: PathBuf,
    rollback: PathBuf,
    had_original: bool,
}

fn sibling_path(target: &Path, kind: &str) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(
        ".{name}.ftb-translater-{kind}-{}-{sequence}",
        std::process::id()
    ))
}

fn create_staging_file(target: &Path) -> io::Result<(PathBuf, std::fs::File)> {
    for _ in 0..1000 {
        let path = sibling_path(target, "tmp");
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "cannot allocate a unique staging file",
    ))
}

fn unused_rollback_path(target: &Path) -> io::Result<PathBuf> {
    for _ in 0..1000 {
        let path = sibling_path(target, "rollback");
        if !path.try_exists()? {
            return Ok(path);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "cannot allocate a unique rollback path",
    ))
}

fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn cleanup_staging(staged: &[StagedOutput]) {
    for output in staged {
        let _ = remove_file_if_exists(&output.temporary);
    }
}

fn rollback_output(output: &StagedOutput, options: &WritebackOptions) -> io::Result<()> {
    if output.had_original {
        remove_file_if_exists(&output.target)?;
        if options.fail_rollback(output.index) {
            return Err(injected_error("output rollback"));
        }
        fs::rename(&output.rollback, &output.target)
    } else {
        remove_file_if_exists(&output.target)
    }
}

pub(crate) fn commit_outputs_with_options(
    outputs: &[FileOutput],
    task_id: &str,
    options: &WritebackOptions,
) -> AppResult<()> {
    let mut unique_paths = HashSet::new();
    if let Some(duplicate) = outputs
        .iter()
        .find(|output| !unique_paths.insert(output.path.clone()))
    {
        let message = format!("输出列表包含重复路径：{}", duplicate.path.display());
        return Err(AppError::commit_failed(message.clone(), message, false));
    }

    let mut staged = Vec::with_capacity(outputs.len());
    // Stage every output before moving any task-book file, so preparation failures are
    // guaranteed to leave the task book untouched.
    for (index, output) in outputs.iter().enumerate() {
        let stage_result = (|| {
            if options.fail_temp_create(index) {
                return Err(injected_error("temporary file create"));
            }
            let (temporary, mut file) = create_staging_file(&output.path)?;
            let prepare_result = (|| {
                file.write_all(output.content.as_bytes())?;
                file.sync_all()?;
                drop(file);
                let metadata = fs::metadata(&output.path);
                let had_original = match metadata {
                    Ok(metadata) => {
                        fs::set_permissions(&temporary, metadata.permissions())?;
                        true
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                    Err(error) => return Err(error),
                };
                if options.fail_temp_prepare(index) {
                    return Err(injected_error("temporary file prepare"));
                }
                Ok(StagedOutput {
                    index,
                    target: output.path.clone(),
                    temporary: temporary.clone(),
                    rollback: unused_rollback_path(&output.path)?,
                    had_original,
                })
            })();
            if prepare_result.is_err() {
                let _ = remove_file_if_exists(&temporary);
            }
            prepare_result
        })();
        match stage_result {
            Ok(output) => staged.push(output),
            Err(error) => {
                cleanup_staging(&staged);
                let message = format!("无法为 {} 创建临时输出：{error}", output.path.display());
                return Err(AppError::commit_failed(message.clone(), message, false));
            }
        }
    }

    let mut committed = Vec::new();
    for (index, output) in staged.iter().enumerate() {
        let mut original_moved = false;
        let replace_result = (|| {
            if output.had_original {
                // Windows rename does not replace an existing destination. Move the old
                // target aside first, then install the same-directory staged file.
                fs::rename(&output.target, &output.rollback)?;
                original_moved = true;
            }
            if options.fail_replace(index) {
                return Err(injected_error("final output replace"));
            }
            fs::rename(&output.temporary, &output.target)
        })();
        if let Err(error) = replace_result {
            let mut rollback_errors = Vec::new();
            if original_moved {
                if let Err(rollback_error) = rollback_output(output, options) {
                    rollback_errors.push(format!("{}：{rollback_error}", output.target.display()));
                }
            }
            for committed_index in committed.iter().rev().copied() {
                let previous = &staged[committed_index];
                if let Err(rollback_error) = rollback_output(previous, options) {
                    rollback_errors
                        .push(format!("{}：{rollback_error}", previous.target.display()));
                }
            }
            cleanup_staging(&staged);
            logging::error(
                "translation",
                "output_commit_failed",
                "输出文件提交失败并已尝试回滚",
                json!({
                    "task_id":task_id,
                    "path":output.target,
                    "committed_before_failure":index,
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
            let message = format!("无法替换 {}：{error}；{rollback}", output.target.display());
            return Err(AppError::commit_failed(
                message,
                format!(
                    "replace failed: {error}; rollback errors: {}",
                    rollback_errors.join("; ")
                ),
                !rollback_errors.is_empty(),
            ));
        }
        committed.push(index);
    }
    for output in &staged {
        if output.had_original {
            if let Err(error) = remove_file_if_exists(&output.rollback) {
                logging::warn(
                    "translation",
                    "rollback_cleanup_failed_after_commit",
                    "任务书已提交，但旧文件清理失败",
                    json!({"task_id":task_id,"path":output.rollback,"error":error.to_string()}),
                );
            }
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
    let task_id = if let Some(task_id) = payload["_task_id"]
        .as_str()
        .filter(|task_id| !task_id.trim().is_empty())
    {
        task_id.to_string()
    } else if document.meta.task_id.trim().is_empty() {
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

struct DryRunSource {
    entries: Vec<Entry>,
    items: Vec<Item>,
    files_to_modify: Vec<String>,
    layout: DryRunLayout,
}

enum DryRunLayout {
    Lang(snbt::LangMap),
    Chapters(Vec<chapters::Segment>),
}

fn load_dry_run_source(q: &Path, mode: &str) -> AppResult<DryRunSource> {
    let mut entries = Vec::new();
    let mut items = Vec::new();
    let mut files_to_modify = Vec::new();
    let layout = if mode == "lang" {
        let map = snbt::load(&q.join("lang/en_us.snbt")).map_err(|message| {
            AppError::source_changed(
                message.clone(),
                format!("reload source language for CMP validation: {message}"),
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
        files_to_modify.push("lang/zh_cn.snbt".into());
        DryRunLayout::Lang(map)
    } else {
        let mut entry_index = 0;
        let mut chapter_segments = Vec::new();
        for file in chapters::files(q) {
            let segments = chapters::extract(&file).map_err(|message| {
                AppError::source_changed(
                    message.clone(),
                    format!("reload chapter for CMP validation: {message}"),
                )
            })?;
            if !segments.is_empty() {
                files_to_modify.push(format!(
                    "chapters/{}",
                    file.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            for segment in segments {
                let (entry, entry_items) = prepare_entry(
                    segment.cache_id.clone(),
                    segment.source.clone(),
                    entry_index,
                    None,
                );
                entries.push(entry);
                items.extend(entry_items);
                chapter_segments.push(segment);
                entry_index += 1;
            }
        }
        DryRunLayout::Chapters(chapter_segments)
    };
    Ok(DryRunSource {
        entries,
        items,
        files_to_modify,
        layout,
    })
}

fn apply_dry_run_edits(document: &mut cmp::Document, edits: &[CmpTargetEdit]) -> AppResult<()> {
    if edits.is_empty() {
        return Ok(());
    }
    if edits.len() != document.records.len() {
        return Err(AppError::invalid_input(
            "校对表格条目数与 CMP 不一致，请重新打开 CMP",
            format!(
                "dry-run edits={} cmp records={}",
                edits.len(),
                document.records.len()
            ),
        ));
    }
    for (expected_index, edit) in edits.iter().enumerate() {
        if edit.index != expected_index {
            return Err(AppError::invalid_input(
                "校对表格内容已过期，请重新打开 CMP",
                format!(
                    "dry-run edit index={} expected={expected_index}",
                    edit.index
                ),
            ));
        }
        document.records[expected_index].target = edit.target.clone();
    }
    Ok(())
}

fn blocked_report(
    belongs_to_current_task_book: bool,
    source_fingerprint_matches: bool,
    blocking_issues: Vec<String>,
) -> CmpValidationReport {
    CmpValidationReport {
        belongs_to_current_task_book,
        source_fingerprint_matches,
        applicable_entries: 0,
        format_guard_failures: 0,
        unchanged_entries: 0,
        files_to_modify: Vec::new(),
        blocking: true,
        blocking_issues,
    }
}

pub fn validate_cmp(payload: &Value, edits: &[CmpTargetEdit]) -> AppResult<CmpValidationReport> {
    let cmp_path = PathBuf::from(
        payload["cmp_path"]
            .as_str()
            .ok_or_else(|| AppError::invalid_input("缺少 CMP 文件路径", "cmp_path is missing"))?,
    );
    let mut document =
        cmp::load(&cmp_path).map_err(|message| AppError::cmp_invalid(message.clone(), message))?;
    apply_dry_run_edits(&mut document, edits)?;
    let selected = Path::new(payload["quests_dir"].as_str().ok_or_else(|| {
        AppError::invalid_input(
            "缺少任务书目录",
            "quests_dir is missing from CMP validation payload",
        )
    })?);
    let q = resolve(selected).map_err(|message| {
        AppError::invalid_input(message.clone(), format!("resolve task book: {message}"))
    })?;
    let current_mode = mode(&q).map_err(|message| {
        AppError::invalid_input(message.clone(), format!("detect task book mode: {message}"))
    })?;
    let source = load_dry_run_source(&q, current_mode)?;

    let identity_error = validate_cmp_identity(&document, &q, current_mode).err();
    let belongs_to_current_task_book = identity_error.is_none();
    let source_fingerprint_matches = document.meta.total_entries == source.entries.len()
        && document.meta.source_fingerprint == source_fingerprint(&source.entries);
    if !belongs_to_current_task_book || !source_fingerprint_matches {
        let mut issues = Vec::new();
        if let Some(error) = identity_error {
            issues.push(error.user_message);
        }
        if !source_fingerprint_matches {
            issues.push("任务书内容或源指纹在 CMP 生成后发生了变化".into());
        }
        return Ok(blocked_report(
            belongs_to_current_task_book,
            source_fingerprint_matches,
            issues,
        ));
    }

    let expected = source
        .items
        .iter()
        .map(|item| ((item.entry_id.as_str(), item.path.as_str()), item))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut unit_targets = HashMap::new();
    let mut structural_issues = Vec::new();
    if document.records.len() != source.items.len() {
        structural_issues.push(format!(
            "CMP 翻译单元数量不一致：CMP 为 {}，当前任务书为 {}",
            document.records.len(),
            source.items.len()
        ));
    }
    for record in &document.records {
        let key = (record.entry_id.as_str(), record.path.as_str());
        if !seen.insert(key) {
            structural_issues.push(format!(
                "CMP 包含重复回填位置：{} {}",
                record.entry_id, record.path
            ));
            continue;
        }
        let Some(item) = expected.get(&key) else {
            structural_issues.push(format!(
                "CMP 包含未知回填位置：{} {}",
                record.entry_id, record.path
            ));
            continue;
        };
        let expected_file = entry_source_file(current_mode, &record.entry_id);
        if record.file != expected_file {
            structural_issues.push(format!(
                "CMP 文件归属被修改：{} 应属于 {}",
                record.entry_id, expected_file
            ));
        }
        if record.source != item.source {
            structural_issues.push(format!(
                "CMP 英文原文被修改：{} {}",
                record.entry_id, record.path
            ));
        }
        if record.target.trim().is_empty() {
            structural_issues.push(format!(
                "CMP 译文不能为空：{} {}",
                record.entry_id, record.path
            ));
        }
        unit_targets.insert(item.id.clone(), record.target.clone());
    }
    for item in &source.items {
        if !seen.contains(&(item.entry_id.as_str(), item.path.as_str())) {
            structural_issues.push(format!("CMP 缺少回填位置：{} {}", item.entry_id, item.path));
        }
    }
    if !structural_issues.is_empty() || unit_targets.len() != source.items.len() {
        return Ok(blocked_report(true, true, structural_issues));
    }

    let mut format_failure_entries = HashSet::new();
    let mut blocking_issues = Vec::new();
    for record in &document.records {
        let problems = warnings(&record.source, &record.target);
        if !problems.is_empty() {
            format_failure_entries.insert(record.entry_id.clone());
            blocking_issues.push(format!(
                "{} {}：{}",
                record.entry_id,
                record.path,
                problems.join("；")
            ));
        }
    }

    let mut applicable_entries = 0;
    let mut unchanged_entries = 0;
    let mut results = HashMap::new();
    for entry in &source.entries {
        let target = match &entry.kind {
            EntryKind::Untouched(_) => entry.source.clone(),
            _ => match render_entry(entry, &unit_targets) {
                Ok(target) => target,
                Err(message) => {
                    format_failure_entries.insert(entry.id.clone());
                    blocking_issues.push(format!("{}：{message}", entry.id));
                    continue;
                }
            },
        };
        let problems = warnings(&entry.source, &target);
        if !problems.is_empty() && format_failure_entries.insert(entry.id.clone()) {
            blocking_issues.push(format!("{}：{}", entry.id, problems.join("；")));
        }
        if target == entry.source {
            unchanged_entries += 1;
        }
        if !format_failure_entries.contains(&entry.id) {
            applicable_entries += 1;
        }
        results.insert(entry.id.clone(), target);
    }

    let format_guard_failures = format_failure_entries.len();
    let mut output_issues = Vec::new();
    if results.len() == source.entries.len() {
        match source.layout {
            DryRunLayout::Lang(mut map) => {
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
                let content = snbt::dump(&map);
                if let Err(message) = snbt::parse(&content) {
                    output_issues.push(format!("lang/zh_cn.snbt：{message}"));
                }
            }
            DryRunLayout::Chapters(segments) => {
                let mut by_file: HashMap<PathBuf, Vec<(usize, String)>> = HashMap::new();
                for segment in segments {
                    by_file
                        .entry(segment.path)
                        .or_default()
                        .push((segment.index, results[&segment.cache_id].clone()));
                }
                for (file, replacements) in by_file {
                    if let Err(message) = chapters::render_replacements(&file, &replacements) {
                        output_issues.push(format!(
                            "chapters/{}：{message}",
                            file.file_name().unwrap_or_default().to_string_lossy()
                        ));
                    }
                }
            }
        }
    }
    blocking_issues.extend(output_issues);
    let blocking = !blocking_issues.is_empty();
    Ok(CmpValidationReport {
        belongs_to_current_task_book: true,
        source_fingerprint_matches: true,
        applicable_entries,
        format_guard_failures,
        unchanged_entries,
        files_to_modify: source.files_to_modify,
        blocking,
        blocking_issues,
    })
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
    apply_cmp_inner_with_options(
        data_dir,
        payload,
        cmp_path,
        document,
        task_id,
        phase,
        validate_only,
        &WritebackOptions::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_cmp_inner_with_options(
    data_dir: Option<&Path>,
    payload: &Value,
    cmp_path: PathBuf,
    document: cmp::Document,
    task_id: &str,
    phase: &std::cell::Cell<&str>,
    validate_only: bool,
    options: &WritebackOptions,
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

    pending_outputs.sort_by(|left, right| left.path.cmp(&right.path));
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
    let backup = backup_with_options(&q, m, options)?;
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
    commit_outputs_with_options(&pending_outputs, task_id, options)?;
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
    let report_value =
        serde_json::to_value(&report).expect("Report contains only values supported by serde_json");
    let data_dir = data_dir.expect("apply mode always provides an application data directory");
    let history_result = if options.fail_history() {
        Err(AppError::history_save_failed(
            "任务书已写入，但翻译历史保存失败",
            "test fault injected at history save",
            true,
        ))
    } else {
        match History::new(data_dir) {
            Ok(history) => history
                .insert(&q, m, &settings, &report_value, &outputs)
                .map_err(|error| error.with_task_book_modified(true)),
            Err(message) => Err(AppError::history_save_failed(
                message.clone(),
                message,
                true,
            )),
        }
    };
    let mut post_commit_warnings = Vec::new();
    let run_id = match history_result {
        Ok(run_id) => run_id,
        Err(error) => {
            logging::warn(
                "translation",
                "history_write_failed_after_commit",
                "任务书已写入，但翻译历史保存失败",
                json!({"task_id":task_id,"quests_dir":q,"error":error}),
            );
            post_commit_warnings
                .push("任务书已成功写入，但翻译历史保存失败；本次运行编号不可用".to_string());
            0
        }
    };
    let report_path = q.join(".ftb-translater/report-latest.json");
    let report_result = if options.fail_report() {
        Err("test fault injected at report save".to_string())
    } else {
        serde_json::to_vec_pretty(&report_value)
            .map_err(|error| error.to_string())
            .and_then(|bytes| fs::write(&report_path, bytes).map_err(|error| error.to_string()))
    };
    if let Err(error) = report_result {
        logging::warn(
            "translation",
            "report_write_failed_after_commit",
            "任务书已写入，但最新报告保存失败",
            json!({"task_id":task_id,"path":report_path,"error":error}),
        );
        post_commit_warnings.push("任务书已成功写入，但最新翻译报告保存失败".to_string());
    }
    logging::info(
        "translation",
        "cmp_applied",
        "CMP 校对结果已写入任务书",
        json!({"task_id":task_id,"cmp_path":cmp_path,"run_id":run_id,"output_files":outputs.len()}),
    );
    Ok(json!({
        "report":report,
        "run_id":run_id,
        "task_id":task_id,
        "post_commit_warnings":post_commit_warnings
    }))
}
