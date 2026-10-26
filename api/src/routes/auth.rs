use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::auth::{self, User};
use crate::db::{uuid_of, Q};
use crate::error::{AppError, Result};
use crate::validate::{Input, Str, EMAIL};
use crate::web::Body;
use crate::AppState;

pub const PASSWORD: Str = Str { trim: false, ..Str::req(0, 200) };

/// Checks a new password: at least 12 and at most 200 characters.
pub fn password(input: &mut Input, key: &str, optional: bool) -> Option<String> {
    let rule = if optional { PASSWORD.optional() } else { PASSWORD };
    let value = input.str(key, rule).into_option()?;
    if value.chars().count() < 12 {
        input.fail(key, "Password must be at least 12 characters");
        return None;
    }
    Some(value)
}

fn with_session(state: &AppState, status: StatusCode, token: &str, body: serde_json::Value) -> Response {
    (status, [(SET_COOKIE, auth::session_cookie(&state.config, token))], Json(body)).into_response()
}

pub async fn signup(State(state): State<AppState>, Body(body): Body) -> Result<Response> {
    if !state.config.allow_signup {
        return Err(AppError::new(403, "Sign-up is disabled. Ask your administrator for an account."));
    }
    let mut input = Input::body(&body);
    let org_name = input.req("organizationName", Str::req(1, 120));
    let name = input.req("name", Str::req(1, 120));
    let email = input.req("email", EMAIL);
    let password = password(&mut input, "password", false);
    input.done()?;
    let hash = auth::hash_password(password.unwrap()).await?;

    let mut tx = state.pool.begin().await?;
    let org =
        Q::new("INSERT INTO organizations (name) VALUES ($1) RETURNING id, name").bind(&org_name).one(&mut tx).await?;
    let mut user = Q::new(
        "INSERT INTO users (org_id, email, name, password_hash, role)
         VALUES ($1, $2::citext, $3, $4, 'admin') RETURNING id, org_id, email, name, role, session_version",
    )
    .bind(uuid_of(&org["id"]))
    .bind(&email)
    .bind(&name)
    .bind(&hash)
    .one(&mut tx)
    .await?;
    tx.commit().await?;
    user["org_name"] = org["name"].clone();
    let u = User::from_row(&user).unwrap();
    let token = auth::sign_session(&state.config, u.id, u.org_id, user["session_version"].as_i64().unwrap() as i32);
    Ok(with_session(&state, StatusCode::CREATED, &token, json!({ "user": u.public() })))
}

pub async fn login(State(state): State<AppState>, Body(body): Body) -> Result<Response> {
    let mut input = Input::body(&body);
    let email = input.req("email", EMAIL);
    let password = input.req("password", Str { trim: false, ..Str::req(1, 200) });
    input.done()?;
    let mut db = state.pool.acquire().await?;
    let row = Q::new(
        "SELECT u.id, u.org_id, u.email, u.name, u.role, u.password_hash, u.is_active, u.session_version,
                o.name AS org_name
           FROM users u JOIN organizations o ON o.id = u.org_id WHERE u.email = $1::citext",
    )
    .bind(&email)
    .opt(&mut db)
    .await?;
    let hash =
        row.as_ref().and_then(|r| r["password_hash"].as_str()).map(str::to_string).unwrap_or_else(auth::dummy_hash);
    let ok = auth::verify_password(password, hash).await;
    let Some(row) = row.filter(|r| ok && r["is_active"] == true) else {
        return Err(AppError::new(401, "Incorrect email or password"));
    };
    let user = User::from_row(&row).unwrap();
    Q::new("UPDATE users SET last_login_at = now() WHERE id = $1").bind(user.id).run(&mut db).await?;
    let token =
        auth::sign_session(&state.config, user.id, user.org_id, row["session_version"].as_i64().unwrap() as i32);
    Ok(with_session(&state, StatusCode::OK, &token, json!({ "user": user.public() })))
}

pub async fn logout(State(state): State<AppState>) -> Response {
    (StatusCode::NO_CONTENT, [(SET_COOKIE, auth::clear_session_cookie(&state.config))]).into_response()
}

pub async fn me(user: User) -> Json<serde_json::Value> {
    Json(json!({ "user": user.public() }))
}

pub async fn change_password(State(state): State<AppState>, user: User, Body(body): Body) -> Result<Response> {
    let mut input = Input::body(&body);
    let current = input.req("currentPassword", Str { trim: false, ..Str::req(1, 200) });
    let new_password = password(&mut input, "newPassword", false);
    input.done()?;
    let mut db = state.pool.acquire().await?;
    let hash: String = Q::new("SELECT password_hash FROM users WHERE id = $1").bind(user.id).scalar(&mut db).await?;
    if !auth::verify_password(current, hash).await {
        return Err(AppError::new(400, "Current password is incorrect"));
    }
    let new_hash = auth::hash_password(new_password.unwrap()).await?;
    let version: i32 = Q::new(
        "UPDATE users SET password_hash = $1, session_version = session_version + 1, updated_at = now()
          WHERE id = $2 RETURNING session_version",
    )
    .bind(new_hash)
    .bind(user.id)
    .scalar(&mut db)
    .await?;
    let token = auth::sign_session(&state.config, user.id, user.org_id, version);
    Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, auth::session_cookie(&state.config, &token))]).into_response())
}
