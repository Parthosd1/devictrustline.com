//! Invitations: an admin invites someone by email to join their workspace with a role. The person
//! accepts by signing in to their existing account (same email) or by creating one.

use axum::extract::{Path, Query, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

use crate::auth::{self, User, ROLES};
use crate::db::{uuid_of, Q};
use crate::error::{AppError, Result};
use crate::mail::{self, Email};
use crate::routes::auth::{issue_token, password, start_session, NewToken};
use crate::routes::users::{check_owner_change, Access};
use crate::tokens;
use crate::validate::{path_uuid, Input, Str, EMAIL};
use crate::web::Body;
use crate::AppState;

const INVITE_HOURS: i32 = 7 * 24;

pub async fn create(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("users:manage")?;
    let mut input = Input::body(&body);
    let email = input.req("email", EMAIL);
    let role = input.one_of("role", ROLES, false, false).into_option();
    let access = Access::read(&mut input).unwrap_or_else(Access::all);
    input.strict()?;
    let role_name = role.clone().unwrap_or_default();
    check_owner_change(&user, &role_name)?;
    access.check_role(&role_name)?;
    let mut tx = state.pool.begin().await?;
    access.check_inventories(&mut tx, user.org_id).await?;
    let member: i64 = Q::new(
        "SELECT count(*) FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.org_id = $1 AND u.email = $2::citext",
    )
    .bind(user.org_id)
    .bind(&email)
    .scalar(&mut tx)
    .await?;
    if member > 0 {
        return Err(AppError::new(409, "This person is already in this workspace. Edit them under Users instead."));
    }
    // A new invitation replaces any earlier one for the same address.
    Q::new(
        "UPDATE auth_tokens SET revoked_at = now()
          WHERE purpose = 'invite' AND org_id = $1 AND email = $2::citext AND used_at IS NULL AND revoked_at IS NULL",
    )
    .bind(user.org_id)
    .bind(&email)
    .run(&mut tx)
    .await?;
    let invite = NewToken {
        purpose: "invite",
        org_id: Some(user.org_id),
        email: &email,
        role: role.as_deref(),
        created_by: Some(user.id),
        hours: INVITE_HOURS,
        ..Default::default()
    };
    let token = issue_token(&mut tx, invite).await?;
    Q::new("UPDATE auth_tokens SET access_scope = $2, inventory_ids = $3 WHERE token_hash = $1")
        .bind(tokens::hash(&token))
        .bind(&access.scope)
        .bind(access.inventories.clone())
        .run(&mut tx)
        .await?;
    let link = format!("{}/?invite={token}", state.config.app_url);
    let emailed = state.config.mail.available();
    if emailed {
        let body = format!(
            "{} invited you to join {} on DeviceTrustline as {}.\n\nAccept the invitation here:\n{link}\n\nThe link works once and expires in 7 days.",
            user.name,
            user.org_name,
            role.as_deref().unwrap_or("")
        );
        let subject = format!("Join {} on DeviceTrustline", user.org_name);
        let message = Email {
            org_id: Some(user.org_id),
            user_id: None,
            to: &email,
            subject: &subject,
            body: &body,
            kind: "invite",
            dedupe_key: None,
        };
        mail::enqueue(&mut tx, &state.config.mail, message).await?;
    }
    tx.commit().await?;
    // Without an email provider the admin gets the link to pass on themselves.
    let mut res = json!({
        "email": email, "role": role, "emailed": emailed,
        "accessScope": access.scope, "inventoryIds": access.inventories,
    });
    if !emailed {
        res["link"] = json!(link);
    }
    Ok((StatusCode::CREATED, Json(res)))
}

pub async fn list(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("users:read")?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(
        r#"SELECT t.id, t.email::text AS email, t.role, t.created_at AS "createdAt", t.expires_at AS "expiresAt",
                  u.name AS "invitedBy", COALESCE(t.access_scope, 'all') AS "accessScope",
                  to_json(COALESCE(t.inventory_ids, '{}')) AS "inventoryIds"
             FROM auth_tokens t LEFT JOIN users u ON u.id = t.created_by
            WHERE t.purpose = 'invite' AND t.org_id = $1 AND t.used_at IS NULL AND t.revoked_at IS NULL AND t.expires_at > now()
            ORDER BY t.created_at DESC"#,
    )
    .bind(user.org_id)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn revoke(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<StatusCode> {
    user.require("users:manage")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let n = Q::new(
        "UPDATE auth_tokens SET revoked_at = now()
          WHERE id = $1 AND org_id = $2 AND purpose = 'invite' AND used_at IS NULL AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(user.org_id)
    .run(&mut db)
    .await?;
    if n == 0 {
        return Err(AppError::not_found("Invitation"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// What an invitation link is for, without using it up.
pub async fn lookup(
    State(state): State<AppState>,
    parts: Parts,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let token = q.get("token").map(String::as_str).unwrap_or("");
    let mut db = state.pool.acquire().await?;
    let row = Q::new(
        "SELECT o.name AS workspace, o.kind, t.email::text AS email, t.role,
                EXISTS (SELECT 1 FROM users u WHERE u.email = t.email) AS \"accountExists\"
           FROM auth_tokens t JOIN organizations o ON o.id = t.org_id
          WHERE t.token_hash = $1 AND t.purpose = 'invite' AND t.used_at IS NULL AND t.revoked_at IS NULL AND t.expires_at > now()",
    )
    .bind(tokens::hash(token))
    .opt(&mut db)
    .await?
    .ok_or_else(|| AppError::coded(404, "token_invalid", "This invitation has expired, was revoked or was already used."))?;
    let signed_in = auth::authenticate(&parts, &state).await.ok();
    let mut row = row;
    row["signedInAs"] = json!(signed_in.map(|u| u.email));
    Ok(Json(row))
}

/// Accepts an invitation. An existing account must be signed in (proving it is theirs); otherwise
/// a new account is created for the invited address.
pub async fn accept(State(state): State<AppState>, parts: Parts, Body(body): Body) -> Result<Response> {
    let mut input = Input::body(&body);
    let token = input.req("token", Str::req(1, 200));
    let name = input.str("name", Str::req(1, 120).optional()).into_option();
    let new_password = password(&mut input, "password", true);
    input.strict()?;
    let signed_in = auth::authenticate(&parts, &state).await.ok();
    let hash = match new_password {
        Some(p) => Some(auth::hash_password(p).await?),
        None => None,
    };

    let mut tx = state.pool.begin().await?;
    let invite = crate::routes::auth::consume_token(&mut tx, "invite", &token).await?.ok_or_else(|| {
        AppError::coded(400, "token_invalid", "This invitation has expired, was revoked or was already used.")
    })?;
    let org = uuid_of(&invite["org_id"]).unwrap();
    crate::routes::plans::check_limit(&mut tx, org, "members").await?;
    let email = invite["email"].as_str().unwrap_or("").to_string();
    let existing: Option<Uuid> =
        Q::new("SELECT id FROM users WHERE email = $1::citext").bind(&email).scalars(&mut tx).await?.into_iter().next();

    let user_id = match (existing, signed_in) {
        (Some(id), Some(me)) if me.id == id => id,
        (Some(_), _) => {
            return Err(AppError::coded(
                409,
                "sign_in_required",
                "An account with this email already exists. Sign in to it first, then open the invitation again.",
            ));
        }
        (None, _) => {
            let (Some(name), Some(hash)) = (name, hash) else {
                return Err(AppError::new(400, "Enter your name and choose a password to create your account"));
            };
            // An emailed link proves the person controls the address.
            let verified = state.config.mail.available();
            Q::new(
                "INSERT INTO users (email, name, password_hash, email_verified_at)
                 VALUES ($1::citext, $2, $3, CASE WHEN $4 THEN now() END) RETURNING id",
            )
            .bind(&email)
            .bind(name)
            .bind(hash)
            .bind(verified)
            .scalar(&mut tx)
            .await?
        }
    };
    // The access the invitation gave; inventories archived or removed since are left out, and an
    // invitation whose inventories are all gone gives no access rather than everything.
    let access = Q::new(
        "SELECT COALESCE(t.access_scope, 'all') AS scope,
                to_json(ARRAY(SELECT i.id FROM inventories i
                               WHERE i.org_id = t.org_id AND i.id = ANY(t.inventory_ids) AND i.archived_at IS NULL)) AS ids
           FROM auth_tokens t WHERE t.id = $1",
    )
    .bind(uuid_of(&invite["id"]))
    .one(&mut tx)
    .await?;
    let inventories: Vec<Uuid> =
        access["ids"].as_array().map(|a| a.iter().filter_map(uuid_of).collect()).unwrap_or_default();
    let access = Access { scope: access["scope"].as_str().unwrap_or("all").to_string(), inventories };
    if access.scope == "inventories" && access.inventories.is_empty() {
        return Err(AppError::coded(
            409,
            "invite_inventories_gone",
            "The inventories this invitation gave access to no longer exist. Ask for a new invitation.",
        ));
    }
    // Someone rejoining keeps no access from before: the membership is reset to what was invited.
    Q::new("UPDATE memberships SET access_scope = 'all' WHERE org_id = $1 AND user_id = $2")
        .bind(org)
        .bind(user_id)
        .run(&mut tx)
        .await?;
    Q::new(
        "INSERT INTO memberships (org_id, user_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (org_id, user_id) DO UPDATE SET role = EXCLUDED.role, is_active = true",
    )
    .bind(org)
    .bind(user_id)
    .bind(invite["role"].as_str())
    .run(&mut tx)
    .await?;
    access.apply(&mut tx, org, user_id, uuid_of(&invite["created_by"])).await?;
    let res = start_session(&state, &mut tx, user_id, org, StatusCode::OK).await?;
    tx.commit().await?;
    Ok(res)
}

/// Local testing only (MAIL_MODE=capture): the messages the server would have sent.
pub async fn dev_mailbox(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    if !matches!(state.config.mail, crate::config::MailConfig::Capture { .. }) {
        return Err(AppError::new(404, "Not found"));
    }
    let mut db = state.pool.acquire().await?;
    let items = Q::new(
        r#"SELECT id, to_address::text AS "to", subject, body_text AS body, kind, status, attempts,
                  last_error AS "lastError", created_at AS "createdAt", sent_at AS "sentAt"
             FROM email_outbox WHERE ($1::text IS NULL OR to_address = $1::citext) ORDER BY id DESC LIMIT 100"#,
    )
    .bind(q.get("to"))
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}
