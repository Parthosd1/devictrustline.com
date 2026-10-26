//! Accounts and sessions: sign-up (personal or business), sign-in, workspaces and switching,
//! email confirmation, password reset and password change.

use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{self, SignedIn, User};
use crate::db::{Db, Q};
use crate::error::{AppError, Result};
use crate::mail::{self, Email};
use crate::tokens;
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

/// Signs the account in to one of its workspaces: sets the session cookie and remembers the
/// workspace as the most recently used.
pub async fn start_session(
    state: &AppState,
    db: &mut Db,
    user_id: Uuid,
    org_id: Uuid,
    status: StatusCode,
) -> Result<Response> {
    let user = auth::load_user(db, user_id, org_id).await?.ok_or_else(|| AppError::not_found("Workspace"))?;
    let version: i32 = Q::new("SELECT session_version FROM users WHERE id = $1").bind(user_id).scalar(&mut *db).await?;
    Q::new("UPDATE memberships SET last_used_at = now() WHERE org_id = $1 AND user_id = $2")
        .bind(org_id)
        .bind(user_id)
        .run(&mut *db)
        .await?;
    let token = auth::sign_session(&state.config, user_id, org_id, version);
    let cookie = auth::session_cookie(&state.config, &token);
    Ok((status, [(SET_COOKIE, cookie)], Json(json!({ "user": user.public() }))).into_response())
}

/// Creates a workspace with `user_id` as its admin, plus its starting categories and reminder rules.
pub async fn create_workspace(db: &mut Db, user_id: Uuid, name: &str, kind: &str) -> Result<Uuid> {
    let plan = if kind == "personal" { "personal_basic" } else { "business" };
    let org: Uuid = Q::new("INSERT INTO organizations (name, kind, plan) VALUES ($1, $2, $3) RETURNING id")
        .bind(name)
        .bind(kind)
        .bind(plan)
        .scalar(&mut *db)
        .await?;
    Q::new("INSERT INTO memberships (org_id, user_id, role) VALUES ($1, $2, 'admin')")
        .bind(org)
        .bind(user_id)
        .run(&mut *db)
        .await?;
    crate::routes::catalog::seed_workspace(db, org, kind).await?;
    Ok(org)
}

/// A one-time link to issue: what it is for, and who and what it applies to.
#[derive(Default)]
pub struct NewToken<'a> {
    pub purpose: &'a str,
    pub user_id: Option<Uuid>,
    pub org_id: Option<Uuid>,
    pub email: &'a str,
    pub role: Option<&'a str>,
    pub created_by: Option<Uuid>,
    pub hours: i32,
}

/// Issues a one-time token and returns the raw token; only its hash is stored.
pub async fn issue_token(db: &mut Db, t: NewToken<'_>) -> Result<String> {
    let token = tokens::new_token();
    Q::new(
        "INSERT INTO auth_tokens (purpose, token_hash, user_id, org_id, email, role, created_by, expires_at)
         VALUES ($1, $2, $3, $4, $5::citext, $6, $7, now() + make_interval(hours => $8))",
    )
    .bind(t.purpose)
    .bind(tokens::hash(&token))
    .bind(t.user_id)
    .bind(t.org_id)
    .bind(t.email)
    .bind(t.role)
    .bind(t.created_by)
    .bind(t.hours)
    .run(db)
    .await?;
    Ok(token)
}

/// Uses up a token: returns its row once, and never again. Expired or revoked tokens return None.
pub async fn consume_token(db: &mut Db, purpose: &str, token: &str) -> Result<Option<Value>> {
    Q::new(
        "UPDATE auth_tokens SET used_at = now()
          WHERE token_hash = $1 AND purpose = $2 AND used_at IS NULL AND revoked_at IS NULL AND expires_at > now()
          RETURNING id, user_id, org_id, email::text AS email, role",
    )
    .bind(tokens::hash(token))
    .bind(purpose)
    .opt(db)
    .await
}

async fn send_verification(state: &AppState, db: &mut Db, user_id: Uuid, email: &str) -> Result<()> {
    Q::new(
        "UPDATE auth_tokens SET revoked_at = now() WHERE user_id = $1 AND purpose = 'verify_email' AND used_at IS NULL",
    )
    .bind(user_id)
    .run(&mut *db)
    .await?;
    let token = issue_token(
        db,
        NewToken { purpose: "verify_email", user_id: Some(user_id), email, hours: 48, ..Default::default() },
    )
    .await?;
    let link = format!("{}/?verify={token}", state.config.app_url);
    let body = format!(
        "Confirm your email address for DeviceTrustline by opening this link:\n\n{link}\n\nThe link works once and expires in 48 hours. If you did not create an account, ignore this message."
    );
    let email = Email {
        org_id: None,
        user_id: Some(user_id),
        to: email,
        subject: "Confirm your DeviceTrustline email",
        body: &body,
        kind: "verify_email",
        dedupe_key: None,
    };
    mail::enqueue(db, &state.config.mail, email).await?;
    Ok(())
}

pub async fn config(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "signup": state.config.allow_signup,
        "google": state.config.google.is_some(),
        "email": state.config.mail.available(),
        "emailVerificationRequired": state.config.require_email_verification,
    }))
}

pub async fn signup(State(state): State<AppState>, Body(body): Body) -> Result<Response> {
    if !state.config.allow_signup {
        return Err(AppError::new(403, "Sign-up is disabled. Ask your administrator for an account."));
    }
    let mut input = Input::body(&body);
    let kind = input.one_of_or("accountType", &["personal", "business"], "business");
    let org_rule = if kind == "personal" { Str::req(1, 120).optional() } else { Str::req(1, 120) };
    let org_name = input.str("organizationName", org_rule).into_option();
    let name = input.req("name", Str::req(1, 120));
    let email = input.req("email", EMAIL);
    let password = password(&mut input, "password", false);
    input.done()?;
    let hash = auth::hash_password(password.unwrap()).await?;
    let org_name = org_name.unwrap_or_else(|| format!("{name}'s things"));

    let mut tx = state.pool.begin().await?;
    let user_id: Uuid =
        Q::new("INSERT INTO users (email, name, password_hash) VALUES ($1::citext, $2, $3) RETURNING id")
            .bind(&email)
            .bind(&name)
            .bind(&hash)
            .scalar(&mut tx)
            .await?;
    let org = create_workspace(&mut tx, user_id, &org_name, &kind).await?;
    if state.config.mail.available() {
        send_verification(&state, &mut tx, user_id, &email).await?;
    }
    let res = start_session(&state, &mut tx, user_id, org, StatusCode::CREATED).await?;
    tx.commit().await?;
    Ok(res)
}

pub async fn login(State(state): State<AppState>, Body(body): Body) -> Result<Response> {
    let mut input = Input::body(&body);
    let email = input.req("email", EMAIL);
    let password = input.req("password", Str { trim: false, ..Str::req(1, 200) });
    input.done()?;
    let mut db = state.pool.acquire().await?;
    let row = Q::new("SELECT id, password_hash, is_active FROM users WHERE email = $1::citext")
        .bind(&email)
        .opt(&mut db)
        .await?;
    let hash = row.as_ref().and_then(|r| r["password_hash"].as_str()).map(str::to_string);
    let ok = auth::verify_password(password, hash.clone().unwrap_or_else(auth::dummy_hash)).await && hash.is_some();
    let wrong = || AppError::new(401, "Incorrect email or password");
    let Some(row) = row.filter(|r| ok && r["is_active"] == true) else { return Err(wrong()) };
    let user_id = crate::db::uuid_of(&row["id"]).unwrap();
    // The workspace used most recently, among those the person still belongs to.
    let org: Option<Uuid> = Q::new(
        "SELECT org_id FROM memberships WHERE user_id = $1 AND is_active
          ORDER BY last_used_at DESC NULLS LAST, created_at LIMIT 1",
    )
    .bind(user_id)
    .scalars(&mut db)
    .await?
    .into_iter()
    .next();
    let Some(org) = org else { return Err(wrong()) };
    Q::new("UPDATE users SET last_login_at = now() WHERE id = $1").bind(user_id).run(&mut db).await?;
    start_session(&state, &mut db, user_id, org, StatusCode::OK).await
}

pub async fn logout(State(state): State<AppState>) -> Response {
    (StatusCode::NO_CONTENT, [(SET_COOKIE, auth::clear_session_cookie(&state.config))]).into_response()
}

pub async fn me(SignedIn(user): SignedIn) -> Json<Value> {
    Json(json!({ "user": user.public() }))
}

pub async fn change_password(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Body(body): Body,
) -> Result<Response> {
    let mut input = Input::body(&body);
    let current = input.req("currentPassword", Str { trim: false, ..Str::req(1, 200) });
    let new_password = password(&mut input, "newPassword", false);
    input.done()?;
    let mut db = state.pool.acquire().await?;
    let hash: Option<String> =
        Q::new("SELECT password_hash FROM users WHERE id = $1").bind(user.id).scalar(&mut db).await?;
    if !auth::verify_password(current, hash.unwrap_or_else(auth::dummy_hash)).await {
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

/// Every workspace the signed-in person belongs to.
pub async fn workspaces(State(state): State<AppState>, SignedIn(user): SignedIn) -> Result<Json<Value>> {
    let mut db = state.pool.acquire().await?;
    let items = Q::new(
        "SELECT o.id, o.name, o.kind, m.role, o.id = $2 AS current
           FROM memberships m JOIN organizations o ON o.id = m.org_id
          WHERE m.user_id = $1 AND m.is_active ORDER BY o.name",
    )
    .bind(user.id)
    .bind(user.org_id)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn create_workspace_route(
    State(state): State<AppState>,
    user: User,
    Body(body): Body,
) -> Result<(StatusCode, Json<Value>)> {
    let mut input = Input::body(&body);
    let name = input.req("name", Str::req(1, 120));
    let kind = input.one_of("kind", &["personal", "business"], false, false).into_option().unwrap_or_default();
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let id = create_workspace(&mut tx, user.id, &name, &kind).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "name": name, "kind": kind, "role": "admin", "current": false }))))
}

pub async fn switch(State(state): State<AppState>, SignedIn(user): SignedIn, Body(body): Body) -> Result<Response> {
    let mut input = Input::body(&body);
    let org = input.uuid("workspaceId", false, false).into_option();
    input.strict()?;
    let mut db = state.pool.acquire().await?;
    start_session(&state, &mut db, user.id, org.unwrap(), StatusCode::OK).await
}

pub async fn resend_verification(State(state): State<AppState>, SignedIn(user): SignedIn) -> Result<Json<Value>> {
    if user.email_verified {
        return Ok(Json(json!({ "emailVerified": true })));
    }
    if !state.config.mail.available() {
        return Err(AppError::coded(503, "email_unavailable", "Email is not set up on this server yet."));
    }
    let mut db = state.pool.acquire().await?;
    send_verification(&state, &mut db, user.id, &user.email).await?;
    Ok(Json(json!({ "sent": true })))
}

pub async fn verify_email(State(state): State<AppState>, Body(body): Body) -> Result<Json<Value>> {
    let mut input = Input::body(&body);
    let token = input.req("token", Str::req(1, 200));
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let row = consume_token(&mut tx, "verify_email", &token)
        .await?
        .ok_or_else(|| AppError::coded(400, "token_invalid", "This link has expired or was already used."))?;
    // The address must still be the one the link was sent to.
    Q::new(
        "UPDATE users SET email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1 AND email = $2::citext",
    )
    .bind(crate::db::uuid_of(&row["user_id"]))
    .bind(row["email"].as_str())
    .run(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "emailVerified": true })))
}

pub async fn request_password_reset(
    State(state): State<AppState>,
    Body(body): Body,
) -> Result<(StatusCode, Json<Value>)> {
    let mut input = Input::body(&body);
    let email = input.req("email", EMAIL);
    input.strict()?;
    let available = state.config.mail.available();
    if available {
        let mut tx = state.pool.begin().await?;
        let user: Option<Uuid> = Q::new("SELECT id FROM users WHERE email = $1::citext AND is_active")
            .bind(&email)
            .scalars(&mut tx)
            .await?
            .into_iter()
            .next();
        // The response is the same whether or not the account exists.
        if let Some(user_id) = user {
            Q::new("UPDATE auth_tokens SET revoked_at = now() WHERE user_id = $1 AND purpose = 'reset_password' AND used_at IS NULL")
                .bind(user_id)
                .run(&mut tx)
                .await?;
            let token = issue_token(
                &mut tx,
                NewToken {
                    purpose: "reset_password",
                    user_id: Some(user_id),
                    email: &email,
                    hours: 1,
                    ..Default::default()
                },
            )
            .await?;
            let link = format!("{}/?reset={token}", state.config.app_url);
            let body = format!(
                "Someone asked to reset the password for your DeviceTrustline account.\n\nChoose a new password here:\n{link}\n\nThe link works once and expires in 1 hour. If this was not you, ignore this message; your password has not changed."
            );
            let message = Email {
                org_id: None,
                user_id: Some(user_id),
                to: &email,
                subject: "Reset your DeviceTrustline password",
                body: &body,
                kind: "reset_password",
                dedupe_key: None,
            };
            mail::enqueue(&mut tx, &state.config.mail, message).await?;
        }
        tx.commit().await?;
    }
    Ok((StatusCode::ACCEPTED, Json(json!({ "emailAvailable": available }))))
}

pub async fn reset_password(State(state): State<AppState>, Body(body): Body) -> Result<StatusCode> {
    let mut input = Input::body(&body);
    let token = input.req("token", Str::req(1, 200));
    let new_password = password(&mut input, "password", false);
    input.strict()?;
    let hash = auth::hash_password(new_password.unwrap()).await?;
    let mut tx = state.pool.begin().await?;
    let row = consume_token(&mut tx, "reset_password", &token)
        .await?
        .ok_or_else(|| AppError::coded(400, "token_invalid", "This link has expired or was already used."))?;
    let user_id = crate::db::uuid_of(&row["user_id"]);
    // A new password signs the account out everywhere; the link also proved control of the inbox.
    Q::new(
        "UPDATE users SET password_hash = $2, session_version = session_version + 1, updated_at = now(),
                email_verified_at = COALESCE(email_verified_at, now())
          WHERE id = $1",
    )
    .bind(user_id)
    .bind(hash)
    .run(&mut tx)
    .await?;
    Q::new("UPDATE auth_tokens SET revoked_at = now() WHERE user_id = $1 AND purpose = 'reset_password' AND used_at IS NULL")
        .bind(user_id)
        .run(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn account(State(state): State<AppState>, SignedIn(user): SignedIn) -> Result<Json<Value>> {
    let mut db = state.pool.acquire().await?;
    let row = Q::new(
        "SELECT u.name, u.email, u.email_notifications AS \"emailNotifications\",
                u.password_hash IS NOT NULL AS \"hasPassword\", u.email_verified_at IS NOT NULL AS \"emailVerified\",
                EXISTS (SELECT 1 FROM user_identities i WHERE i.user_id = u.id AND i.provider = 'google') AS \"googleLinked\"
           FROM users u WHERE u.id = $1",
    )
    .bind(user.id)
    .one(&mut db)
    .await?;
    Ok(Json(row))
}

pub async fn update_account(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Body(body): Body,
) -> Result<Json<Value>> {
    let mut input = Input::body(&body);
    let name = input.str("name", Str::req(1, 120).optional()).into_option();
    let notify = input.boolean("emailNotifications", false).into_option();
    input.strict()?;
    let mut db = state.pool.acquire().await?;
    Q::new(
        "UPDATE users SET name = COALESCE($2, name), email_notifications = COALESCE($3, email_notifications), updated_at = now()
          WHERE id = $1",
    )
    .bind(user.id)
    .bind(name)
    .bind(notify)
    .run(&mut db)
    .await?;
    drop(db);
    account(State(state), SignedIn(user)).await
}
