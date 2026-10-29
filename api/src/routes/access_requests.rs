//! Requests for access to an inventory. Someone whose access is limited to some inventories asks
//! for another one; a manager or admin with access to everything approves (which adds the grant)
//! or rejects it. Nobody decides their own request.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

use crate::auth::User;
use crate::db::{Db, Q};
use crate::error::{AppError, Result};
use crate::validate::{path_uuid, Input, Str};
use crate::web::Body;
use crate::AppState;

const SELECT: &str = r#"
  SELECT r.id, r.user_id AS "userId", u.name AS "userName", r.inventory_id AS "inventoryId", i.name AS "inventoryName",
         r.note, r.status, r.created_at AS "createdAt", r.decided_by AS "decidedBy", d.name AS "decidedByName",
         r.decided_at AS "decidedAt", r.decision_note AS "decisionNote"
    FROM access_requests r JOIN users u ON u.id = r.user_id JOIN inventories i ON i.id = r.inventory_id
    LEFT JOIN users d ON d.id = r.decided_by"#;

async fn find(db: &mut Db, org: Uuid, id: Uuid) -> Result<Value> {
    Q::new(format!("{SELECT} WHERE r.org_id = $1 AND r.id = $2"))
        .bind(org)
        .bind(id)
        .opt(db)
        .await?
        .ok_or_else(|| AppError::not_found("Access request"))
}

/// Whether this person decides access requests: the permission, and access to everything.
fn decides(user: &User) -> bool {
    crate::auth::can(&user.role, "access:approve") && user.scope.is_none()
}

/// Inventories someone with limited access can ask for (names only), and their own requests.
pub async fn list(
    State(state): State<AppState>,
    user: User,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    user.require("inventory:read")?;
    let mut input = Input::query(&q);
    let status = input.one_of("status", &["pending", "approved", "rejected", "cancelled"], true, false).into_option();
    input.done()?;
    let mut db = state.pool.acquire().await?;
    // Deciders see everyone's requests; everyone else sees their own.
    let mine = if decides(&user) { None } else { Some(user.id) };
    let items = Q::new(format!(
        "{SELECT} WHERE r.org_id = $1 AND ($2::uuid IS NULL OR r.user_id = $2) AND ($3::text IS NULL OR r.status = $3)
          ORDER BY r.created_at DESC LIMIT 500"
    ))
    .bind(user.org_id)
    .bind(mine)
    .bind(status)
    .all(&mut db)
    .await?;
    let requestable = if user.scope.is_some() {
        Q::new(
            r#"SELECT id, name, kind FROM inventories WHERE org_id = $1 AND archived_at IS NULL AND NOT (id = ANY($2))
                ORDER BY name"#,
        )
        .bind(user.org_id)
        .bind(user.scope_param().unwrap_or_default())
        .all(&mut db)
        .await?
    } else {
        vec![]
    };
    Ok(Json(json!({ "items": items, "requestable": requestable, "canDecide": decides(&user) })))
}

pub async fn create(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("inventory:read")?;
    let mut input = Input::body(&body);
    let inventory = input.uuid("inventoryId", false, false).into_option().unwrap_or_default();
    let note = input.str("note", Str::text(1000)).into_option();
    input.strict()?;
    if user.scope.is_none() {
        return Err(AppError::new(409, "You already have access to every inventory"));
    }
    if user.sees_inventory(Some(inventory)) {
        return Err(AppError::new(409, "You already have access to this inventory"));
    }
    let mut tx = state.pool.begin().await?;
    let active: Option<bool> = Q::new("SELECT archived_at IS NULL FROM inventories WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(inventory)
        .scalars(&mut tx)
        .await?
        .into_iter()
        .next();
    if active != Some(true) {
        return Err(AppError::new(400, "Inventory not found"));
    }
    let id: Uuid = Q::new(
        "INSERT INTO access_requests (org_id, user_id, inventory_id, note) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(user.org_id)
    .bind(user.id)
    .bind(inventory)
    .bind(note)
    .scalar(&mut tx)
    .await
    .map_err(|e| {
        if e.constraint() == Some("access_requests_one_pending") {
            AppError::new(409, "You already asked for access to this inventory")
        } else {
            e
        }
    })?;
    let created = find(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    crate::events::access_requested(&state, user.org_id, id).await;
    Ok((StatusCode::CREATED, Json(created)))
}

/// Locks a pending request for a decision.
async fn pending(db: &mut Db, org: Uuid, id: Uuid) -> Result<Value> {
    let r =
        Q::new("SELECT user_id, inventory_id, status FROM access_requests WHERE org_id = $1 AND id = $2 FOR UPDATE")
            .bind(org)
            .bind(id)
            .opt(&mut *db)
            .await?
            .ok_or_else(|| AppError::not_found("Access request"))?;
    if r["status"] != "pending" {
        return Err(AppError::new(409, "This request was already decided"));
    }
    Ok(r)
}

async fn decide(state: AppState, user: User, id: String, body: Value, approve: bool) -> Result<Json<Value>> {
    if !decides(&user) {
        return Err(AppError::new(403, "You do not have permission to do that"));
    }
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let note = input.str("note", Str::text(1000)).into_option();
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let r = pending(&mut tx, user.org_id, id).await?;
    let requester = crate::db::uuid_of(&r["user_id"]).unwrap();
    if requester == user.id {
        return Err(AppError::new(403, "Someone else has to decide your own request"));
    }
    let status = if approve { "approved" } else { "rejected" };
    Q::new(
        "UPDATE access_requests SET status = $3, decided_by = $4, decided_at = now(), decision_note = $5
          WHERE org_id = $1 AND id = $2",
    )
    .bind(user.org_id)
    .bind(id)
    .bind(status)
    .bind(user.id)
    .bind(note)
    .run(&mut tx)
    .await?;
    if approve {
        // Access is only ever added to someone whose access is limited; those with access to
        // everything already reach the inventory.
        Q::new(
            "INSERT INTO inventory_grants (org_id, user_id, inventory_id, created_by)
             SELECT m.org_id, m.user_id, $3, $4 FROM memberships m
              WHERE m.org_id = $1 AND m.user_id = $2 AND m.is_active AND m.access_scope = 'inventories'
                AND EXISTS (SELECT 1 FROM inventories WHERE org_id = $1 AND id = $3 AND archived_at IS NULL)
             ON CONFLICT DO NOTHING",
        )
        .bind(user.org_id)
        .bind(requester)
        .bind(crate::db::uuid_of(&r["inventory_id"]))
        .bind(user.id)
        .run(&mut tx)
        .await?;
    }
    let decided = find(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    crate::events::access_decided(&state, user.org_id, id).await;
    Ok(Json(decided))
}

pub async fn approve(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    decide(state, user, id, body, true).await
}

pub async fn reject(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    decide(state, user, id, body, false).await
}

/// The person who asked withdraws their request.
pub async fn cancel(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    let id = path_uuid(&id)?;
    let mut tx = state.pool.begin().await?;
    let r = pending(&mut tx, user.org_id, id).await?;
    if crate::db::uuid_of(&r["user_id"]) != Some(user.id) {
        return Err(AppError::not_found("Access request"));
    }
    Q::new(
        "UPDATE access_requests SET status = 'cancelled', decided_by = $3, decided_at = now() WHERE org_id = $1 AND id = $2",
    )
    .bind(user.org_id)
    .bind(id)
    .bind(user.id)
    .run(&mut tx)
    .await?;
    let cancelled = find(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok(Json(cancelled))
}
