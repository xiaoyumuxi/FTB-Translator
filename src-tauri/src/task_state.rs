use crate::{
    cmp,
    error::{AppError, AppResult},
    logging,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex, MutexGuard, OnceLock},
};

const DATABASE_FILENAME: &str = "task-state.sqlite3";

static STATE_TRANSITION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static PROCESS_STARTED_AT: LazyLock<chrono::DateTime<Utc>> = LazyLock::new(Utc::now);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Created,
    Translating,
    ReviewReady,
    Applying,
    Applied,
    Failed,
}

impl TaskState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Translating => "translating",
            Self::ReviewReady => "review_ready",
            Self::Applying => "applying",
            Self::Applied => "applied",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> AppResult<Self> {
        match value {
            "created" => Ok(Self::Created),
            "translating" => Ok(Self::Translating),
            "review_ready" => Ok(Self::ReviewReady),
            "applying" => Ok(Self::Applying),
            "applied" => Ok(Self::Applied),
            "failed" => Ok(Self::Failed),
            other => Err(state_storage_error(format!(
                "unknown persisted task state: {other}"
            ))),
        }
    }

    pub fn can_apply(self) -> bool {
        self == Self::ReviewReady
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskIdentity {
    pub id: String,
    pub task_id: String,
    pub quests_dir: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskStatus {
    pub task_id: String,
    pub state: TaskState,
    pub can_apply: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ActiveTask {
    pub task_id: String,
    pub state: TaskState,
    pub updated_at: String,
    pub recoverable: bool,
}

#[derive(Clone, Debug)]
pub struct TaskStateStore {
    database_path: PathBuf,
}

impl TaskStateStore {
    pub fn new(data_dir: &Path) -> AppResult<Self> {
        let _ = &*PROCESS_STARTED_AT;
        std::fs::create_dir_all(data_dir).map_err(|error| {
            state_storage_error(format!(
                "create application data directory {}: {error}",
                data_dir.display()
            ))
        })?;
        let store = Self {
            database_path: data_dir.join(DATABASE_FILENAME),
        };
        let _guard = transition_lock()?;
        store.connection()?;
        Ok(store)
    }

    pub fn identity_for_cmp(&self, document: &cmp::Document) -> AppResult<TaskIdentity> {
        let quests_dir = normalize_existing_path(Path::new(&document.meta.quests_dir))?;
        let task_id = document.meta.task_id.trim();
        let id = if task_id.is_empty() {
            stable_legacy_identity(document, &quests_dir)
        } else {
            task_id.to_string()
        };
        Ok(TaskIdentity {
            task_id: id.clone(),
            id,
            quests_dir,
        })
    }

    pub fn reserve_new_translation(
        &self,
        quests_dir: &Path,
        task_id: &str,
    ) -> AppResult<TaskIdentity> {
        let quests_dir = normalize_existing_path(quests_dir)?;
        let identity = TaskIdentity {
            id: task_id.to_string(),
            task_id: task_id.to_string(),
            quests_dir,
        };
        let _guard = transition_lock()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(sql_error)?;
        reject_busy_task_book(&transaction, &identity.quests_dir, None)?;
        insert_state(&transaction, &identity, TaskState::Created)?;
        set_state(
            &transaction,
            &identity.id,
            TaskState::Created,
            TaskState::Translating,
        )?;
        transaction.commit().map_err(sql_error)?;
        log_transition(&identity.id, TaskState::Created, TaskState::Translating);
        Ok(identity)
    }

    pub fn reserve_retry_translation(&self, document: &cmp::Document) -> AppResult<TaskIdentity> {
        let identity = self.identity_for_cmp(document)?;
        let _guard = transition_lock()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(sql_error)?;
        reject_busy_task_book(&transaction, &identity.quests_dir, Some(&identity.id))?;
        let state = checked_state(&transaction, &identity)?;
        match state {
            None => {
                insert_state(&transaction, &identity, TaskState::ReviewReady)?;
                set_state(
                    &transaction,
                    &identity.id,
                    TaskState::ReviewReady,
                    TaskState::Translating,
                )?;
            }
            Some(TaskState::ReviewReady | TaskState::Failed) => {
                update_state(&transaction, &identity.id, TaskState::Translating)?;
            }
            Some(current) => return Err(transition_error("重试翻译", current)),
        }
        transaction.commit().map_err(sql_error)?;
        logging::info(
            "task_state",
            "task_state_transitioned",
            "任务状态已更新",
            serde_json::json!({
                "task_id":&identity.task_id,
                "to":TaskState::Translating.as_str(),
                "operation":"retry_translation"
            }),
        );
        Ok(identity)
    }

    pub fn translation_succeeded(&self, identity: &str) -> AppResult<()> {
        self.transition(identity, TaskState::Translating, TaskState::ReviewReady)
    }

    pub fn translation_failed(&self, identity: &str) -> AppResult<()> {
        self.transition(identity, TaskState::Translating, TaskState::Failed)
    }

    pub fn register_cmp(&self, document: &cmp::Document) -> AppResult<(TaskIdentity, TaskStatus)> {
        let identity = self.identity_for_cmp(document)?;
        let _guard = transition_lock()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(sql_error)?;
        let state = match checked_state(&transaction, &identity)? {
            Some(state) => state,
            None => {
                insert_state(&transaction, &identity, TaskState::ReviewReady)?;
                TaskState::ReviewReady
            }
        };
        transaction.commit().map_err(sql_error)?;
        logging::debug(
            "task_state",
            "cmp_state_loaded",
            "CMP 任务状态已加载",
            serde_json::json!({"task_id":&identity.task_id,"state":state.as_str()}),
        );
        Ok((identity.clone(), status(&identity, state)))
    }

    pub fn begin_apply(&self, document: &cmp::Document) -> AppResult<TaskIdentity> {
        let identity = self.identity_for_cmp(document)?;
        let _guard = transition_lock()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(sql_error)?;
        let state = match checked_state(&transaction, &identity)? {
            Some(state) => state,
            None => {
                insert_state(&transaction, &identity, TaskState::ReviewReady)?;
                TaskState::ReviewReady
            }
        };
        if state != TaskState::ReviewReady {
            return Err(transition_error("应用 CMP", state));
        }
        reject_busy_task_book(&transaction, &identity.quests_dir, Some(&identity.id))?;
        set_state(
            &transaction,
            &identity.id,
            TaskState::ReviewReady,
            TaskState::Applying,
        )?;
        transaction.commit().map_err(sql_error)?;
        log_transition(&identity.id, TaskState::ReviewReady, TaskState::Applying);
        Ok(identity)
    }

    pub fn apply_succeeded(&self, identity: &str) -> AppResult<()> {
        self.transition(identity, TaskState::Applying, TaskState::Applied)
    }

    pub fn apply_failed(&self, identity: &str, task_book_modified: bool) -> AppResult<()> {
        self.transition(
            identity,
            TaskState::Applying,
            if task_book_modified {
                TaskState::Failed
            } else {
                TaskState::ReviewReady
            },
        )
    }

    pub fn recover_interrupted_translation(&self, quests_dir: &Path) -> AppResult<usize> {
        self.recover_interrupted_translation_before(quests_dir, &PROCESS_STARTED_AT)
    }

    pub fn active_operations(&self, quests_dir: &Path) -> AppResult<Vec<ActiveTask>> {
        let quests_dir = normalize_existing_path(quests_dir)?;
        let _guard = transition_lock()?;
        let connection = self.connection()?;
        let mut query = connection
            .prepare(
                "SELECT task_id,state,updated_at FROM task_states
                 WHERE quests_dir=? AND state IN ('translating','applying')
                 ORDER BY updated_at,task_id",
            )
            .map_err(sql_error)?;
        let rows = query
            .query_map(params![quests_dir], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(sql_error)?;
        let activities = rows
            .map(|row| {
                let (task_id, state, updated_at) = row.map_err(sql_error)?;
                let state = TaskState::parse(&state)?;
                let timestamp = chrono::DateTime::parse_from_rfc3339(&updated_at)
                    .map_err(|error| {
                        state_storage_error(format!("invalid task timestamp: {error}"))
                    })?
                    .with_timezone(&Utc);
                Ok(ActiveTask {
                    task_id,
                    state,
                    updated_at,
                    recoverable: state == TaskState::Translating && timestamp < *PROCESS_STARTED_AT,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        logging::debug(
            "task_state",
            "active_operations_inspected",
            "已检查任务书的活动任务状态",
            serde_json::json!({
                "quests_dir":quests_dir,
                "activities":activities.iter().map(|activity| serde_json::json!({
                    "task_id":activity.task_id,
                    "state":activity.state.as_str(),
                    "recoverable":activity.recoverable
                })).collect::<Vec<_>>()
            }),
        );
        Ok(activities)
    }

    fn recover_interrupted_translation_before(
        &self,
        quests_dir: &Path,
        cutoff: &chrono::DateTime<Utc>,
    ) -> AppResult<usize> {
        let quests_dir = normalize_existing_path(quests_dir)?;
        let _guard = transition_lock()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(sql_error)?;
        let candidates = {
            let mut query = transaction
                .prepare(
                    "SELECT identity,updated_at FROM task_states
                     WHERE quests_dir=? AND state='translating'",
                )
                .map_err(sql_error)?;
            let rows = query
                .query_map(params![quests_dir], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(sql_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(sql_error)?
        };
        let mut recovered = Vec::new();
        for (identity, updated_at) in candidates {
            let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at)
                .map_err(|error| state_storage_error(format!("invalid task timestamp: {error}")))?
                .with_timezone(&Utc);
            if updated_at >= *cutoff {
                continue;
            }
            set_state(
                &transaction,
                &identity,
                TaskState::Translating,
                TaskState::Failed,
            )?;
            recovered.push(identity);
        }
        transaction.commit().map_err(sql_error)?;
        if recovered.is_empty() {
            return Err(AppError::task_state_conflict(
                "没有可恢复的中断翻译任务；当前任务可能仍在运行",
                format!("no translating task older than process start for {quests_dir}"),
            ));
        }
        logging::warn(
            "task_state",
            "interrupted_translations_recovered",
            "用户确认将上次进程中断的翻译任务标记为失败",
            serde_json::json!({"quests_dir":quests_dir,"count":recovered.len()}),
        );
        Ok(recovered.len())
    }

    fn transition(&self, identity: &str, from: TaskState, to: TaskState) -> AppResult<()> {
        let _guard = transition_lock()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(sql_error)?;
        set_state(&transaction, identity, from, to)?;
        transaction.commit().map_err(sql_error)?;
        log_transition(identity, from, to);
        Ok(())
    }

    fn connection(&self) -> AppResult<Connection> {
        let connection = Connection::open(&self.database_path).map_err(sql_error)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=FULL;
                 CREATE TABLE IF NOT EXISTS task_states(
                    identity TEXT PRIMARY KEY,
                    task_id TEXT NOT NULL,
                    quests_dir TEXT NOT NULL,
                    state TEXT NOT NULL CHECK(state IN ('created','translating','review_ready','applying','applied','failed')),
                    updated_at TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS task_states_quests_state
                    ON task_states(quests_dir,state);",
            )
            .map_err(sql_error)?;
        Ok(connection)
    }
}

fn status(identity: &TaskIdentity, state: TaskState) -> TaskStatus {
    TaskStatus {
        task_id: identity.task_id.clone(),
        state,
        can_apply: state.can_apply(),
    }
}

fn log_transition(identity: &str, from: TaskState, to: TaskState) {
    logging::info(
        "task_state",
        "task_state_transitioned",
        "任务状态已更新",
        serde_json::json!({
            "task_id":identity,
            "from":from.as_str(),
            "to":to.as_str()
        }),
    );
}

fn normalize_existing_path(path: &Path) -> AppResult<String> {
    path.canonicalize()
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| {
            AppError::cmp_invalid(
                format!("任务书目录不可用：{error}"),
                format!("canonicalize task book {}: {error}", path.display()),
            )
        })
}

fn stable_legacy_identity(document: &cmp::Document, quests_dir: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"ftb-translater-legacy-cmp-v1\0");
    hash.update(quests_dir.as_bytes());
    hash.update([0]);
    hash.update(document.meta.mode.as_bytes());
    hash.update([0]);
    hash.update(document.meta.source_fingerprint.as_bytes());
    hash.update([0]);
    hash.update(document.meta.total_entries.to_le_bytes());
    for record in &document.records {
        for value in [
            record.file.as_str(),
            record.entry_id.as_str(),
            record.path.as_str(),
            record.source.as_str(),
        ] {
            hash.update(value.as_bytes());
            hash.update([0xff]);
        }
    }
    format!("legacy-{}", hex::encode(hash.finalize()))
}

fn transition_lock() -> AppResult<MutexGuard<'static, ()>> {
    STATE_TRANSITION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| state_storage_error("task state transition lock is poisoned"))
}

fn insert_state(
    transaction: &Transaction<'_>,
    identity: &TaskIdentity,
    state: TaskState,
) -> AppResult<()> {
    transaction
        .execute(
            "INSERT INTO task_states(identity,task_id,quests_dir,state,updated_at)
             VALUES(?,?,?,?,?)",
            params![
                identity.id,
                identity.task_id,
                identity.quests_dir,
                state.as_str(),
                Utc::now().to_rfc3339()
            ],
        )
        .map_err(sql_error)?;
    Ok(())
}

fn set_state(
    transaction: &Transaction<'_>,
    identity: &str,
    from: TaskState,
    to: TaskState,
) -> AppResult<()> {
    let changed = transaction
        .execute(
            "UPDATE task_states SET state=?,updated_at=? WHERE identity=? AND state=?",
            params![
                to.as_str(),
                Utc::now().to_rfc3339(),
                identity,
                from.as_str()
            ],
        )
        .map_err(sql_error)?;
    if changed != 1 {
        let current = state_in_transaction(transaction, identity)?;
        return Err(match current {
            Some(current) => transition_error("更新任务状态", current),
            None => state_storage_error(format!("task state record is missing: {identity}")),
        });
    }
    Ok(())
}

fn update_state(transaction: &Transaction<'_>, identity: &str, to: TaskState) -> AppResult<()> {
    transaction
        .execute(
            "UPDATE task_states SET state=?,updated_at=? WHERE identity=?",
            params![to.as_str(), Utc::now().to_rfc3339(), identity],
        )
        .map_err(sql_error)?;
    Ok(())
}

fn state_in_transaction(
    transaction: &Transaction<'_>,
    identity: &str,
) -> AppResult<Option<TaskState>> {
    state_query(transaction, identity)
}

fn checked_state(
    transaction: &Transaction<'_>,
    identity: &TaskIdentity,
) -> AppResult<Option<TaskState>> {
    let record = transaction
        .query_row(
            "SELECT task_id,quests_dir,state FROM task_states WHERE identity=?",
            params![identity.id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;
    let Some((task_id, quests_dir, state)) = record else {
        return Ok(None);
    };
    if task_id != identity.task_id || quests_dir != identity.quests_dir {
        return Err(AppError::cmp_invalid(
            "CMP 的任务身份与已保存状态冲突，已拒绝继续操作",
            format!(
                "identity={} stored_task_id={} incoming_task_id={} stored_quests_dir={} incoming_quests_dir={}",
                identity.id, task_id, identity.task_id, quests_dir, identity.quests_dir
            ),
        ));
    }
    TaskState::parse(&state).map(Some)
}

fn state_query(connection: &Connection, identity: &str) -> AppResult<Option<TaskState>> {
    let value = connection
        .query_row(
            "SELECT state FROM task_states WHERE identity=?",
            params![identity],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sql_error)?;
    value.map(|value| TaskState::parse(&value)).transpose()
}

fn reject_busy_task_book(
    transaction: &Transaction<'_>,
    quests_dir: &str,
    except_identity: Option<&str>,
) -> AppResult<()> {
    let busy = transaction
        .query_row(
            "SELECT identity,state FROM task_states
             WHERE quests_dir=? AND state IN ('translating','applying')
               AND (? IS NULL OR identity<>?) LIMIT 1",
            params![quests_dir, except_identity, except_identity],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(sql_error)?;
    if let Some((identity, state)) = busy {
        let user_message = if state == TaskState::Applying.as_str() {
            "当前任务书存在未完成的写回状态。为避免重复覆盖，应用不会自动解锁；请先检查任务书、备份和诊断日志"
        } else {
            "当前任务书已有翻译任务正在执行，或上次翻译异常中断；请先检查任务状态"
        };
        return Err(AppError::task_state_conflict(
            user_message,
            format!("busy identity={identity} state={state} quests_dir={quests_dir}"),
        ));
    }
    Ok(())
}

fn transition_error(action: &str, state: TaskState) -> AppError {
    let message = match state {
        TaskState::Translating => "翻译任务已在运行，请勿重复启动",
        TaskState::Applying => "CMP 正在写回，请勿重复应用",
        TaskState::Applied => "该 CMP 已经应用，不能再次写回",
        TaskState::Failed => "任务处于失败状态，请先重新导入或重试翻译",
        TaskState::Created | TaskState::ReviewReady => "任务当前状态不允许此操作",
    };
    AppError::task_state_conflict(
        message,
        format!("action={action} current_state={}", state.as_str()),
    )
}

fn sql_error(error: rusqlite::Error) -> AppError {
    state_storage_error(error.to_string())
}

fn state_storage_error(message: impl Into<String>) -> AppError {
    let message = message.into();
    AppError::task_state_save_failed("无法保存任务状态，请检查应用数据目录后重试", message, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers;
    use std::sync::{Arc, Barrier};
    use tempfile::tempdir;

    fn document(quests_dir: &Path, task_id: &str, target: &str) -> cmp::Document {
        cmp::Document {
            meta: cmp::Meta {
                version: 1,
                task_id: task_id.into(),
                quests_dir: quests_dir.display().to_string(),
                mode: "lang".into(),
                source_fingerprint: "source-fingerprint".into(),
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
                target: target.into(),
                status: "translated".into(),
            }],
        }
    }

    #[test]
    fn concurrent_translation_reservations_allow_only_one_for_a_task_book() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("quests");
        let data_dir = directory.path().join("data");
        std::fs::create_dir_all(&quests).unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let handles = ["first", "second"].map(|task_id| {
            let barrier = barrier.clone();
            let quests = quests.clone();
            let data_dir = data_dir.clone();
            std::thread::spawn(move || {
                let store = TaskStateStore::new(&data_dir).unwrap();
                barrier.wait();
                store.reserve_new_translation(&quests, task_id)
            })
        });
        barrier.wait();
        let results = handles.map(|handle| handle.join().unwrap());
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let error = results
            .iter()
            .find_map(|result| result.as_ref().err())
            .unwrap();
        assert_eq!(error.code, crate::error::ErrorCode::TaskStateConflict);
    }

    #[test]
    fn failed_translation_can_retry_with_the_same_cmp_task_id() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("quests");
        std::fs::create_dir_all(&quests).unwrap();
        let store = TaskStateStore::new(&directory.path().join("data")).unwrap();
        let identity = store
            .reserve_new_translation(&quests, "translation-task")
            .unwrap();
        store.translation_failed(&identity.id).unwrap();

        let retry = store
            .reserve_retry_translation(&document(&quests, "translation-task", "你好"))
            .unwrap();
        assert_eq!(retry.task_id, "translation-task");
        store.translation_succeeded(&retry.id).unwrap();
        let (_, status) = store
            .register_cmp(&document(&quests, "translation-task", "你好"))
            .unwrap();
        assert_eq!(status.state, TaskState::ReviewReady);
        assert!(status.can_apply);
    }

    #[test]
    fn explicit_recovery_marks_only_an_older_translation_as_failed() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("quests");
        std::fs::create_dir_all(&quests).unwrap();
        let store = TaskStateStore::new(&directory.path().join("data")).unwrap();
        store
            .reserve_new_translation(&quests, "interrupted-task")
            .unwrap();

        let cutoff = Utc::now() + chrono::Duration::seconds(1);
        assert_eq!(
            store
                .recover_interrupted_translation_before(&quests, &cutoff)
                .unwrap(),
            1
        );
        store
            .reserve_new_translation(&quests, "replacement-task")
            .unwrap();
    }

    #[test]
    fn active_operations_distinguish_recoverable_translation_from_writeback() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("quests");
        std::fs::create_dir_all(&quests).unwrap();
        let store = TaskStateStore::new(&directory.path().join("data")).unwrap();
        let interrupted = store
            .reserve_new_translation(&quests, "interrupted-task")
            .unwrap();
        store
            .connection()
            .unwrap()
            .execute(
                "UPDATE task_states SET updated_at=? WHERE identity=?",
                params![
                    (*PROCESS_STARTED_AT - chrono::Duration::minutes(1)).to_rfc3339(),
                    interrupted.id
                ],
            )
            .unwrap();

        let active = store.active_operations(&quests).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].state, TaskState::Translating);
        assert!(active[0].recoverable);
        store.recover_interrupted_translation(&quests).unwrap();

        let cmp = document(&quests, "writeback-task", "你好");
        store.begin_apply(&cmp).unwrap();
        let active = store.active_operations(&quests).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].task_id, "writeback-task");
        assert_eq!(active[0].state, TaskState::Applying);
        assert!(!active[0].recoverable);
    }

    #[test]
    fn successful_apply_is_remembered_after_ui_loss_and_restart() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("quests");
        std::fs::create_dir_all(&quests).unwrap();
        let data_dir = directory.path().join("data");
        let cmp = document(&quests, "apply-task", "你好");
        let store = TaskStateStore::new(&data_dir).unwrap();
        let (_, imported) = store.register_cmp(&cmp).unwrap();
        assert_eq!(imported.state, TaskState::ReviewReady);
        let identity = store.begin_apply(&cmp).unwrap();
        store.apply_succeeded(&identity.id).unwrap();

        let duplicate = store.begin_apply(&cmp).unwrap_err();
        assert_eq!(duplicate.code, crate::error::ErrorCode::TaskStateConflict);

        let restarted = TaskStateStore::new(&data_dir).unwrap();
        let (_, imported_again) = restarted.register_cmp(&cmp).unwrap();
        assert_eq!(imported_again.state, TaskState::Applied);
        assert!(!imported_again.can_apply);
        assert!(restarted.begin_apply(&cmp).is_err());
    }

    #[test]
    fn apply_failures_restore_only_safe_retries() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("quests");
        std::fs::create_dir_all(&quests).unwrap();
        let store = TaskStateStore::new(&directory.path().join("data")).unwrap();

        let safe = document(&quests, "safe-failure", "你好");
        let safe_identity = store.begin_apply(&safe).unwrap();
        store.apply_failed(&safe_identity.id, false).unwrap();
        assert!(store.begin_apply(&safe).is_ok());

        let modified = document(&quests, "modified-failure", "你好");
        let modified_identity = store.begin_apply(&modified).unwrap_err();
        assert_eq!(
            modified_identity.code,
            crate::error::ErrorCode::TaskStateConflict
        );
        // Finish the first in-flight retry so this task book can start another apply.
        store.apply_failed(&safe_identity.id, false).unwrap();
        let modified_identity = store.begin_apply(&modified).unwrap();
        store.apply_failed(&modified_identity.id, true).unwrap();
        let error = store.begin_apply(&modified).unwrap_err();
        assert_eq!(error.code, crate::error::ErrorCode::TaskStateConflict);
    }

    #[test]
    fn old_v1_identity_is_stable_across_target_edits_and_store_restarts() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("quests");
        std::fs::create_dir_all(&quests).unwrap();
        let data_dir = directory.path().join("data");
        let mut original = document(&quests, "", "你好");
        let first_store = TaskStateStore::new(&data_dir).unwrap();
        let first = first_store.identity_for_cmp(&original).unwrap();
        assert!(first.id.starts_with("legacy-"));
        original.records[0].target = "您好".into();
        original.records[0].status = "review".into();

        let restarted = TaskStateStore::new(&data_dir).unwrap();
        let second = restarted.identity_for_cmp(&original).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn state_database_does_not_persist_cmp_text_or_credentials() {
        let directory = tempdir().unwrap();
        let quests = directory.path().join("quests");
        std::fs::create_dir_all(&quests).unwrap();
        let data_dir = directory.path().join("data");
        let store = TaskStateStore::new(&data_dir).unwrap();
        let mut cmp = document(&quests, "privacy-task", "UNIQUE_TARGET_SECRET");
        cmp.records[0].source = "UNIQUE_SOURCE_SECRET".into();
        store.register_cmp(&cmp).unwrap();
        drop(store);

        for entry in std::fs::read_dir(&data_dir).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_file() {
                continue;
            }
            let bytes = std::fs::read(entry.path()).unwrap();
            for forbidden in [
                b"UNIQUE_SOURCE_SECRET".as_slice(),
                b"UNIQUE_TARGET_SECRET".as_slice(),
                b"UNIQUE_API_KEY_SECRET".as_slice(),
            ] {
                assert!(!bytes
                    .windows(forbidden.len())
                    .any(|window| window == forbidden));
            }
        }
    }
}
