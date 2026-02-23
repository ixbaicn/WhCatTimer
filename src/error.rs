use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
  InvalidArgument,
  UnsupportedCronSyntax,
  InvalidCronExpression,
  JobLimitExceeded,
  PayloadTooLarge,
  JobNotFound,
  StoreError,
  Internal,
}

impl ErrorCode {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::InvalidArgument => "E_INVALID_ARGUMENT",
      Self::UnsupportedCronSyntax => "E_UNSUPPORTED_CRON_SYNTAX",
      Self::InvalidCronExpression => "E_INVALID_CRON_EXPRESSION",
      Self::JobLimitExceeded => "E_JOB_LIMIT_EXCEEDED",
      Self::PayloadTooLarge => "E_PAYLOAD_TOO_LARGE",
      Self::JobNotFound => "E_JOB_NOT_FOUND",
      Self::StoreError => "E_STORE_ERROR",
      Self::Internal => "E_INTERNAL",
    }
  }
}

#[derive(Debug, Clone)]
pub struct TimerError {
  pub code: ErrorCode,
  pub message: String,
}

impl TimerError {
  pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
    Self {
      code,
      message: message.into(),
    }
  }
}

impl Display for TimerError {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}: {}", self.code.as_str(), self.message)
  }
}

impl std::error::Error for TimerError {}

impl From<std::io::Error> for TimerError {
  fn from(value: std::io::Error) -> Self {
    Self::new(ErrorCode::StoreError, value.to_string())
  }
}

pub type TimerResult<T> = Result<T, TimerError>;
