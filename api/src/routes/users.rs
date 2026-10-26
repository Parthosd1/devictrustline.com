use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use crate::auth::{self, User, ROLES};
use crate::db::Q;
use crate::error::{AppError, Result};
use crate::routes::auth::password;
use crate::validate::{path_uuid, Input, Str, EMAIL};
use crate::web::Body;
use crate::AppState;

const COLUMNS: &str =
    r#"id, email, name, role, is_active AS "isActive", last_login_at AS "lastLoginAt", created_at AS "createdAt""#;

pub async fn list(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("users:read")?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!("SELECT {COLUMNS} FROM users WHERE org_id = $1 ORDER BY name"))
        .bind(user.org_id)
        .all(&mut db)
        .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn create(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("users:manage")?;
    let mut input = Input::body(&body);
    let name = input.req("name", Str::req(1, 120));
    let email = input.req("email", EMAIL);
    let role = input.one_of("role", ROLES, false, false);
    // Temporary password the admin shares with the new user.
    let password = password(&mut input, "password", false);
    input.done()?;
    let hash = auth::hash_password(password.unwrap()).await?;
    let mut db = state.pool.acquire().await?;
    let created = Q::new(format!(
        "INSERT INTO users (org_id, email, name, password_hash, role) VALUES ($1, $2::citext, $3, $4, $5) RETURNING {COLUMNS}"
    ))
    .bind(user.org_id)
    .bind(email)
    .bind(name)
    .bind(hash)
    .bind(role.into_option())
    .one(&mut db)
    .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("users:manage")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let name = input.str("name", Str::req(1, 120).optional());
    let role = input.one_of("role", ROLES, true, false);
    let is_active = input.boolean("isActive", false);
    let password = password(&mut input, "password", true);
    input.strict()?;
    let hash = match password {
        Some(p) => Some(auth::hash_password(p).await?),
        None => None,
    };

    let mut tx = state.pool.begin().await?;
    // Lock the org's admins so two concurrent edits cannot remove the last one.
    let admins = Q::new("SELECT id FROM users WHERE org_id = $1 AND role = 'admin' AND is_active FOR UPDATE")
        .bind(user.org_id)
        .all(&mut tx)
        .await?;
    let current = Q::new("SELECT role, is_active FROM users WHERE id = $1 AND org_id = $2")
        .bind(id)
        .bind(user.org_id)
        .opt(&mut tx)
        .await?
        .ok_or_else(|| AppError::not_found("User"))?;
    let current_admin = current["role"] == "admin" && current["is_active"] == true;
    let next_role = role.value().map(String::as_str).or(current["role"].as_str()).unwrap_or("");
    let next_active = is_active.value().copied().unwrap_or(current["is_active"] == true);
    if current_admin && !(next_role == "admin" && next_active) && admins.len() <= 1 {
        return Err(AppError::new(409, "An organization needs at least one active admin"));
    }
    let updated = Q::new(format!(
        "UPDATE users SET
           name = COALESCE($3, name),
           role = COALESCE($4, role),
           is_active = COALESCE($5, is_active),
           password_hash = COALESCE($6, password_hash),
           session_version = session_version + CASE WHEN $6::text IS NULL THEN 0 ELSE 1 END,
           updated_at = now()
         WHERE id = $1 AND org_id = $2 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(user.org_id)
    .bind(name.into_option())
    .bind(role.into_option())
    .bind(is_active.into_option())
    .bind(hash)
    .one(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(updated))
}
