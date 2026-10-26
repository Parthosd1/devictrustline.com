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

const COLUMNS: &str = r#"u.id, u.email, u.name, m.role, (m.is_active AND u.is_active) AS "isActive",
    u.last_login_at AS "lastLoginAt", m.created_at AS "createdAt""#;

async fn member(db: &mut crate::db::Db, org_id: uuid::Uuid, id: uuid::Uuid) -> Result<Value> {
    Q::new(format!(
        "SELECT {COLUMNS} FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.org_id = $1 AND m.user_id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .opt(db)
    .await?
    .ok_or_else(|| AppError::not_found("User"))
}

pub async fn list(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("users:read")?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!(
        "SELECT {COLUMNS} FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.org_id = $1 ORDER BY u.name"
    ))
    .bind(user.org_id)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

/// Creates an account with a temporary password the admin shares. People who already have an
/// account (in any workspace) are invited instead, so nobody can take over an existing login.
pub async fn create(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("users:manage")?;
    let mut input = Input::body(&body);
    let name = input.req("name", Str::req(1, 120));
    let email = input.req("email", EMAIL);
    let role = input.one_of("role", ROLES, false, false);
    let password = password(&mut input, "password", false);
    input.done()?;
    let hash = auth::hash_password(password.unwrap()).await?;
    let mut tx = state.pool.begin().await?;
    let id: Option<uuid::Uuid> = Q::new(
        "INSERT INTO users (email, name, password_hash) VALUES ($1::citext, $2, $3) ON CONFLICT (email) DO NOTHING RETURNING id",
    )
    .bind(&email)
    .bind(&name)
    .bind(hash)
    .scalars(&mut tx)
    .await?
    .into_iter()
    .next();
    let Some(id) = id else {
        return Err(AppError::coded(
            409,
            "account_exists",
            "This email already has a DeviceTrustline account. Send them an invitation instead.",
        ));
    };
    Q::new("INSERT INTO memberships (org_id, user_id, role) VALUES ($1, $2, $3)")
        .bind(user.org_id)
        .bind(id)
        .bind(role.into_option())
        .run(&mut tx)
        .await?;
    let created = member(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
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
    // Lock the workspace's admins so two concurrent edits cannot remove the last one.
    let admins =
        Q::new("SELECT user_id FROM memberships WHERE org_id = $1 AND role = 'admin' AND is_active FOR UPDATE")
            .bind(user.org_id)
            .all(&mut tx)
            .await?;
    let current = Q::new("SELECT role, is_active FROM memberships WHERE user_id = $1 AND org_id = $2")
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
    if name.is_present() || hash.is_some() {
        // The account itself (name, password) belongs to its owner once they use other workspaces.
        let others: i64 = Q::new("SELECT count(*) FROM memberships WHERE user_id = $1 AND org_id <> $2")
            .bind(id)
            .bind(user.org_id)
            .scalar(&mut tx)
            .await?;
        if others > 0 && id != user.id {
            return Err(AppError::new(
                403,
                "This person also uses other workspaces, so only they can change their name or password",
            ));
        }
        Q::new(
            "UPDATE users SET
               name = COALESCE($2, name),
               password_hash = COALESCE($3, password_hash),
               session_version = session_version + CASE WHEN $3::text IS NULL THEN 0 ELSE 1 END,
               updated_at = now()
             WHERE id = $1",
        )
        .bind(id)
        .bind(name.into_option())
        .bind(hash)
        .run(&mut tx)
        .await?;
    }
    Q::new("UPDATE memberships SET role = COALESCE($3, role), is_active = COALESCE($4, is_active) WHERE user_id = $1 AND org_id = $2")
        .bind(id)
        .bind(user.org_id)
        .bind(role.into_option())
        .bind(is_active.into_option())
        .run(&mut tx)
        .await?;
    let updated = member(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok(Json(updated))
}
