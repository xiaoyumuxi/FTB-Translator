use serde::Serialize;
use std::fmt;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    UnsupportedFormat,
    SourceChanged,
    CmpInvalid,
    ProviderFailed,
    RateLimited,
    FormatGuardRejected,
    BackupFailed,
    CommitFailed,
    HistorySaveFailed,
}

impl ErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::UnsupportedFormat => "unsupported_format",
            Self::SourceChanged => "source_changed",
            Self::CmpInvalid => "cmp_invalid",
            Self::ProviderFailed => "provider_failed",
            Self::RateLimited => "rate_limited",
            Self::FormatGuardRejected => "format_guard_rejected",
            Self::BackupFailed => "backup_failed",
            Self::CommitFailed => "commit_failed",
            Self::HistorySaveFailed => "history_save_failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub user_message: String,
    pub internal_message: String,
    pub retryable: bool,
    pub task_book_modified: bool,
}

impl AppError {
    pub fn new(
        code: ErrorCode,
        user_message: impl Into<String>,
        internal_message: impl Into<String>,
        retryable: bool,
        task_book_modified: bool,
    ) -> Self {
        Self {
            code,
            user_message: user_message.into(),
            internal_message: internal_message.into(),
            retryable,
            task_book_modified,
        }
    }

    pub fn invalid_input(
        user_message: impl Into<String>,
        internal_message: impl Into<String>,
    ) -> Self {
        Self::new(
            ErrorCode::InvalidInput,
            user_message,
            internal_message,
            false,
            false,
        )
    }

    pub fn with_task_book_modified(mut self, task_book_modified: bool) -> Self {
        self.task_book_modified = task_book_modified;
        self
    }

    pub fn cmp_invalid(
        user_message: impl Into<String>,
        internal_message: impl Into<String>,
    ) -> Self {
        Self::new(
            ErrorCode::CmpInvalid,
            user_message,
            internal_message,
            false,
            false,
        )
    }

    pub fn provider_failed(
        user_message: impl Into<String>,
        internal_message: impl Into<String>,
    ) -> Self {
        Self::new(
            ErrorCode::ProviderFailed,
            user_message,
            internal_message,
            true,
            false,
        )
    }

    pub fn rate_limited(
        user_message: impl Into<String>,
        internal_message: impl Into<String>,
    ) -> Self {
        Self::new(
            ErrorCode::RateLimited,
            user_message,
            internal_message,
            true,
            false,
        )
    }

    pub fn source_changed(
        user_message: impl Into<String>,
        internal_message: impl Into<String>,
    ) -> Self {
        Self::new(
            ErrorCode::SourceChanged,
            user_message,
            internal_message,
            false,
            false,
        )
    }

    pub fn format_guard_rejected(
        user_message: impl Into<String>,
        internal_message: impl Into<String>,
    ) -> Self {
        Self::new(
            ErrorCode::FormatGuardRejected,
            user_message,
            internal_message,
            false,
            false,
        )
    }

    pub fn backup_failed(
        user_message: impl Into<String>,
        internal_message: impl Into<String>,
    ) -> Self {
        Self::new(
            ErrorCode::BackupFailed,
            user_message,
            internal_message,
            true,
            false,
        )
    }

    pub fn commit_failed(
        user_message: impl Into<String>,
        internal_message: impl Into<String>,
        task_book_modified: bool,
    ) -> Self {
        Self::new(
            ErrorCode::CommitFailed,
            user_message,
            internal_message,
            true,
            task_book_modified,
        )
    }

    pub fn history_save_failed(
        user_message: impl Into<String>,
        internal_message: impl Into<String>,
        task_book_modified: bool,
    ) -> Self {
        Self::new(
            ErrorCode::HistorySaveFailed,
            user_message,
            internal_message,
            true,
            task_book_modified,
        )
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.user_message)
    }
}

impl std::error::Error for AppError {}

impl From<AppError> for String {
    fn from(error: AppError) -> Self {
        error.user_message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_codes_and_fields_are_serializable() {
        let error = AppError::new(
            ErrorCode::CommitFailed,
            "无法写入输出",
            "permission denied",
            true,
            true,
        );
        let value = serde_json::to_value(&error).unwrap();
        assert_eq!(error.code.as_str(), "commit_failed");
        assert_eq!(value["code"], "commit_failed");
        assert_eq!(value["user_message"], "无法写入输出");
        assert_eq!(value["internal_message"], "permission denied");
        assert_eq!(value["retryable"], true);
        assert_eq!(value["task_book_modified"], true);
    }

    #[test]
    fn string_compatibility_exposes_only_the_existing_user_message() {
        let error = AppError::cmp_invalid("CMP 文件头无效", "unexpected header bytes");
        assert_eq!(error.to_string(), "CMP 文件头无效");
        assert_eq!(String::from(error), "CMP 文件头无效");
    }

    #[test]
    fn category_helpers_set_retry_and_writeback_semantics() {
        let rate_limit = AppError::rate_limited("HTTP 429", "quota");
        assert!(rate_limit.retryable);
        assert!(!rate_limit.task_book_modified);

        let history = AppError::history_save_failed("保存历史失败", "disk full", true);
        assert_eq!(history.code, ErrorCode::HistorySaveFailed);
        assert!(history.retryable);
        assert!(history.task_book_modified);
    }
}
