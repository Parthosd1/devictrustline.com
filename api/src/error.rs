use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::validate::Issue;

#[derive(Debug)]
pub enum AppError {
    Http {
        status: StatusCode,
        message: String,
        details: Option<Value>,
    },
    /// An error the web app reacts to by its machine-readable `code`.
    Coded {
        status: StatusCode,
        message: String,
        code: &'static str,
    },
    Invalid(Vec<Issue>),
    Db(sqlx::Error),
    Internal(String),
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;

impl AppError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        AppError::Http { status: StatusCode::from_u16(status).unwrap(), message: message.into(), details: None }
    }

    pub fn with_details(status: u16, message: impl Into<String>, details: Value) -> Self {
        AppError::Http {
            status: StatusCode::from_u16(status).unwrap(),
            message: message.into(),
            details: Some(details),
        }
    }

    pub fn coded(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        AppError::Coded { status: StatusCode::from_u16(status).unwrap(), message: message.into(), code }
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(404, format!("{what} not found"))
    }

    /// The database constraint a failed statement broke, if any.
    pub fn constraint(&self) -> Option<&str> {
        match self {
            AppError::Db(sqlx::Error::Database(e)) => e.constraint(),
            _ => None,
        }
    }

    /// The Postgres SQLSTATE of a failed statement, if any.
    pub fn pg_code(&self) -> Option<String> {
        match self {
            AppError::Db(sqlx::Error::Database(e)) => e.code().map(|c| c.into_owned()),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::Db(err)
    }
}

impl From<Vec<Issue>> for AppError {
    fn from(issues: Vec<Issue>) -> Self {
        AppError::Invalid(issues)
    }
}

fn pg_message(code: &str) -> Option<(StatusCode, &'static str)> {
    Some(match code {
        "23505" => (StatusCode::CONFLICT, "A record with that value already exists"),
        "23503" => (StatusCode::BAD_REQUEST, "A referenced record does not exist"),
        "23514" => (StatusCode::BAD_REQUEST, "A value is not allowed"),
        "22P02" => (StatusCode::BAD_REQUEST, "Invalid identifier"),
        _ => return None,
    })
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Http { status, message, details } => {
                let mut body = json!({ "error": message });
                if let Some(details) = details {
                    body["details"] = details;
                }
                (status, Json(body)).into_response()
            }
            AppError::Coded { status, message, code } => {
                (status, Json(json!({ "error": message, "code": code }))).into_response()
            }
            AppError::Invalid(issues) => {
                (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid request", "details": issues })))
                    .into_response()
            }
            AppError::Db(sqlx::Error::Database(e)) if e.code().as_deref().and_then(pg_message).is_some() => {
                let (status, message) = pg_message(e.code().as_deref().unwrap()).unwrap();
                let mut body = json!({ "error": message });
                if let Some(c) = e.constraint() {
                    body["constraint"] = json!(c);
                }
                (status, Json(body)).into_response()
            }
            AppError::Db(e) => internal(&e.to_string()),
            AppError::Internal(e) => internal(&e),
        }
    }
}

fn internal(message: &str) -> Response {
    eprintln!("{}", json!({ "level": "error", "t": chrono::Utc::now().to_rfc3339(), "error": message }));
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "Something went wrong" }))).into_response()
}
