//! Moving items between inventories. Every move is a transfer recording who moved the item, from
//! where, to where, when and why; components move with their parent. Moves into or out of an
//! inventory that requires approval wait for a manager unless a manager makes them.
//!
//! Item rows are locked (in id order) for the whole move, an item has one inventory column, and
//! at most one pending request per item is allowed by a unique index, so concurrent requests
//! cannot put an item in two places or move it twice from the same starting point.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::User;
use crate::db::{uuid_of, Db, Q};
use crate::error::{AppError, Result};
use crate::routes::assets::{descendant_ids, find_by_code, record_event};
use crate::validate::{path_uuid, Input, Opt, Str};
use crate::web::Body;
use crate::AppState;

const SELECT: &str = r#"
  SELECT t.id, t.batch_id AS "batchId", t.asset_id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName",
         t.from_inventory_id AS "fromInventoryId", fi.name AS "fromInventoryName",
         t.to_inventory_id AS "toInventoryId", ti.name AS "toInventoryName",
         t.via_asset_id AS "viaAssetId", va.asset_tag AS "viaAssetTag", t.status, t.method, t.reason,
         t.requested_by AS "requestedById", ru.name AS "requestedBy", t.requested_at AS "requestedAt",
         t.decided_by AS "decidedById", du.name AS "decidedBy", t.decided_at AS "decidedAt",
         t.decision_note AS "decisionNote", t.completed_at AS "completedAt"
    FROM inventory_transfers t
    JOIN assets a ON a.id = t.asset_id
    LEFT JOIN inventories fi ON fi.id = t.from_inventory_id
    JOIN inventories ti ON ti.id = t.to_inventory_id
    LEFT JOIN assets va ON va.id = t.via_asset_id
    LEFT JOIN users ru ON ru.id = t.requested_by
    LEFT JOIN users du ON du.id = t.decided_by"#;

pub async fn find(db: &mut Db, org: Uuid, id: Uuid) -> Result<Value> {
    Q::new(format!("{SELECT} WHERE t.org_id = $1 AND t.id = $2"))
        .bind(org)
        .bind(id)
        .opt(db)
        .await?
        .ok_or_else(|| AppError::not_found("Transfer"))
}

pub struct Move<'a> {
    pub to: Uuid,
    /// Where the person saw the item: the move is refused if it has been moved since.
    pub expected_from: Opt<Uuid>,
    pub reason: Option<String>,
    pub method: &'a str,
    pub include_components: bool,
}

/// What happened to each requested item.
#[derive(Default)]
pub struct Outcome {
    pub batch_id: Uuid,
    pub completed: Vec<Uuid>,
    pub pending: Vec<Uuid>,
    pub skipped: Vec<Value>,
}

/// Moves items (and their components) to an inventory, or requests the move when it needs
/// approval. Runs inside the caller's transaction.
pub async fn move_assets(db: &mut Db, user: &User, roots: &[Uuid], m: Move<'_>) -> Result<Outcome> {
    let org = user.org_id;
    let target = Q::new(
        "SELECT id, name, requires_approval, archived_at IS NULL AS active, building_id, location_id
           FROM inventories WHERE org_id = $1 AND id = $2",
    )
    .bind(org)
    .bind(m.to)
    .opt(&mut *db)
    .await?
    .ok_or_else(|| AppError::new(400, "Destination inventory not found"))?;
    if !user.sees_inventory(Some(m.to)) {
        return Err(AppError::new(400, "Destination inventory not found"));
    }
    if target["active"] != true {
        return Err(AppError::new(409, "The destination inventory is archived"));
    }
    if let Opt::Val(from) = m.expected_from {
        let exists: i64 = Q::new("SELECT count(*) FROM inventories WHERE org_id = $1 AND id = $2")
            .bind(org)
            .bind(from)
            .scalar(&mut *db)
            .await?;
        if exists == 0 {
            return Err(AppError::new(400, "Source inventory not found"));
        }
    }
    // Every item to move: each root, then its components, each remembering its root.
    let mut plan: Vec<(Uuid, Option<Uuid>)> = vec![];
    for root in roots {
        if plan.iter().any(|(id, _)| id == root) {
            continue;
        }
        plan.push((*root, None));
        if m.include_components {
            for child in descendant_ids(db, org, *root).await? {
                if !plan.iter().any(|(id, _)| *id == child) {
                    plan.push((child, Some(*root)));
                }
            }
        }
    }
    let mut ids: Vec<Uuid> = plan.iter().map(|(id, _)| *id).collect();
    ids.sort();
    let rows = Q::new(
        r#"SELECT a.id, a.asset_tag AS "assetTag", a.status, a.archived_at IS NOT NULL AS archived, a.inventory_id AS "inventoryId",
                  COALESCE(fi.requires_approval, false) AS "fromApproval",
                  EXISTS (SELECT 1 FROM inventory_transfers p WHERE p.asset_id = a.id AND p.status = 'pending') AS "hasPending"
             FROM assets a LEFT JOIN inventories fi ON fi.id = a.inventory_id
            WHERE a.org_id = $1 AND a.id = ANY($2) ORDER BY a.id FOR UPDATE OF a"#,
    )
    .bind(org)
    .bind(&ids)
    .all(&mut *db)
    .await?;
    let by_id: HashMap<Uuid, &Value> = rows.iter().map(|r| (uuid_of(&r["id"]).unwrap(), r)).collect();
    let approver = user.require("transfers:approve").is_ok();
    // A random (version 4) id shared by every transfer of this request.
    let mut bytes: [u8; 16] = rand::random();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut out = Outcome { batch_id: Uuid::from_bytes(bytes), ..Default::default() };
    let skip = |id: Uuid, tag: &Value, reason: &str, out: &mut Outcome| {
        out.skipped.push(json!({ "assetId": id, "assetTag": tag, "reason": reason }));
    };
    for (id, via) in &plan {
        let row = by_id.get(id).filter(|r| user.sees_inventory(uuid_of(&r["inventoryId"])));
        let Some(row) = row else {
            if via.is_none() {
                return Err(AppError::not_found("Asset"));
            }
            continue;
        };
        let current = uuid_of(&row["inventoryId"]);
        // Components only move with their parent.
        if let Some(root) = via {
            if out.skipped.iter().any(|s| uuid_of(&s["assetId"]) == Some(*root)) {
                skip(*id, &row["assetTag"], "Its parent was not moved", &mut out);
                continue;
            }
        }
        if via.is_none() {
            if let Opt::Val(expected) = m.expected_from {
                if current != Some(expected) {
                    skip(*id, &row["assetTag"], "It is no longer in the inventory it was expected in", &mut out);
                    continue;
                }
            }
        }
        if row["archived"] == true || row["status"] == "Retired" {
            skip(*id, &row["assetTag"], "Archived and retired items are not moved", &mut out);
            continue;
        }
        if current == Some(m.to) {
            skip(*id, &row["assetTag"], "Already in that inventory", &mut out);
            continue;
        }
        if row["hasPending"] == true {
            skip(*id, &row["assetTag"], "Another transfer of this item is waiting for approval", &mut out);
            continue;
        }
        let needs_approval = target["requires_approval"] == true || row["fromApproval"] == true;
        let pending = needs_approval && !approver;
        let inserted = Q::new(
            "INSERT INTO inventory_transfers (org_id, batch_id, asset_id, from_inventory_id, to_inventory_id, via_asset_id,
                                              status, method, reason, requested_by, decided_by, decided_at, completed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     CASE WHEN $11 THEN NULL ELSE $10 END, CASE WHEN $11 THEN NULL ELSE now() END,
                     CASE WHEN $11 THEN NULL ELSE now() END)
             RETURNING id",
        )
        .bind(org)
        .bind(out.batch_id)
        .bind(id)
        .bind(current)
        .bind(m.to)
        .bind(via)
        .bind(if pending { "pending" } else { "completed" })
        .bind(m.method)
        .bind(&m.reason)
        .bind(user.id)
        .bind(pending)
        .scalar::<Uuid>(&mut *db)
        .await;
        let transfer = match inserted {
            Err(e) if e.constraint() == Some("inventory_transfers_one_pending") => {
                skip(*id, &row["assetTag"], "Another transfer of this item is waiting for approval", &mut out);
                continue;
            }
            other => other?,
        };
        if pending {
            out.pending.push(transfer);
        } else {
            apply(db, org, user.id, transfer).await?;
            out.completed.push(transfer);
        }
    }
    Ok(out)
}

/// Puts a completed transfer into effect: the item's inventory (and, when the inventory has a
/// place, its building and location) and its history.
async fn apply(db: &mut Db, org: Uuid, actor: Uuid, transfer: Uuid) -> Result<()> {
    let t = Q::new(
        "SELECT t.asset_id, t.from_inventory_id, t.to_inventory_id, t.reason, t.via_asset_id, t.batch_id, t.requested_by,
                i.building_id, i.location_id, a.building_id AS old_building, a.location_id AS old_location
           FROM inventory_transfers t JOIN inventories i ON i.id = t.to_inventory_id JOIN assets a ON a.id = t.asset_id
          WHERE t.id = $1",
    )
    .bind(transfer)
    .one(&mut *db)
    .await?;
    let asset = uuid_of(&t["asset_id"]).unwrap();
    let place = uuid_of(&t["building_id"]);
    Q::new(
        "UPDATE assets SET inventory_id = $3,
                building_id = CASE WHEN $4::uuid IS NULL THEN building_id ELSE $4 END,
                location_id = CASE WHEN $4::uuid IS NULL THEN location_id ELSE $5 END,
                updated_at = now()
          WHERE org_id = $1 AND id = $2",
    )
    .bind(org)
    .bind(asset)
    .bind(uuid_of(&t["to_inventory_id"]))
    .bind(place)
    .bind(uuid_of(&t["location_id"]))
    .run(&mut *db)
    .await?;
    let mut changes = json!({
        "inventoryId": { "from": t["from_inventory_id"], "to": t["to_inventory_id"] },
        "transfer": { "from": null, "to": transfer },
    });
    if place.is_some() {
        if t["old_building"] != t["building_id"] {
            changes["buildingId"] = json!({ "from": t["old_building"], "to": t["building_id"] });
        }
        if t["old_location"] != t["location_id"] {
            changes["locationId"] = json!({ "from": t["old_location"], "to": t["location_id"] });
        }
    }
    if !t["reason"].is_null() {
        changes["reason"] = json!({ "from": null, "to": t["reason"] });
    }
    if !t["via_asset_id"].is_null() {
        changes["via"] = json!({ "from": null, "to": t["via_asset_id"] });
    }
    record_event(db, org, asset, actor, "transferred", changes).await
}

async fn outcome_json(db: &mut Db, org: Uuid, out: &Outcome) -> Result<Value> {
    let ids: Vec<Uuid> = out.completed.iter().chain(out.pending.iter()).copied().collect();
    let rows = Q::new(format!("{SELECT} WHERE t.org_id = $1 AND t.id = ANY($2) ORDER BY t.requested_at, a.asset_tag"))
        .bind(org)
        .bind(ids)
        .all(db)
        .await?;
    let (completed, pending): (Vec<Value>, Vec<Value>) = rows.into_iter().partition(|r| r["status"] == "completed");
    Ok(json!({ "batchId": out.batch_id, "completed": completed, "pending": pending, "skipped": out.skipped }))
}

/// Moves items by id or by scanned code: `{assetIds | codes, toInventoryId, fromInventoryId?,
/// reason?, includeComponents?}`. One item that cannot be moved is a 409; in a bulk move, items
/// that cannot be moved are listed under `skipped` with the reason.
pub async fn create(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("inventory:transfer")?;
    let mut input = Input::body(&body);
    let to = input.uuid("toInventoryId", false, false).into_option();
    let from = input.uuid("fromInventoryId", true, true);
    let reason = input.str("reason", Str::text(500)).into_option();
    let include = input.boolean_or("includeComponents", true);
    let ids = input.raw_value("assetIds");
    let codes = input.raw_value("codes");
    let as_list = |v: &Option<Value>| -> Option<Vec<String>> {
        v.as_ref()?.as_array()?.iter().map(|x| x.as_str().map(str::to_string)).collect()
    };
    let (ids, codes) = (as_list(&ids), as_list(&codes));
    let count = ids.as_ref().map_or(0, Vec::len) + codes.as_ref().map_or(0, Vec::len);
    if !(1..=500).contains(&count) {
        input.fail("assetIds", "Send 1 to 500 items as assetIds or codes");
    }
    let mut parsed = vec![];
    for id in ids.iter().flatten() {
        match crate::validate::parse_uuid(id) {
            Some(u) => parsed.push(u),
            None => input.fail("assetIds", "Invalid uuid"),
        }
    }
    input.strict()?;
    let method = match (codes.is_some(), count > 1) {
        (true, _) => "scan",
        (false, true) => "bulk",
        _ => "manual",
    };
    let mut tx = state.pool.begin().await?;
    for code in codes.iter().flatten() {
        let asset = find_by_code(&mut tx, user.org_id, code).await?;
        parsed.push(uuid_of(&asset["id"]).unwrap());
    }
    let single = parsed.len() == 1;
    let m = Move { to: to.unwrap(), expected_from: from, reason, method, include_components: include };
    let out = move_assets(&mut tx, &user, &parsed, m).await?;
    if single && out.completed.is_empty() && out.pending.is_empty() {
        let reason = out.skipped.first().and_then(|s| s["reason"].as_str()).unwrap_or("Nothing to move").to_string();
        return Err(AppError::new(409, reason));
    }
    let result = outcome_json(&mut tx, user.org_id, &out).await?;
    tx.commit().await?;
    crate::events::transfers_requested(&state, user.org_id, &out.pending).await;
    Ok((StatusCode::CREATED, Json(result)))
}

pub async fn list(
    State(state): State<AppState>,
    user: User,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    user.require("inventory:read")?;
    let mut input = Input::query(&q);
    let status = input.one_of("status", &["pending", "completed", "rejected", "cancelled"], true, false).into_option();
    let inventory = input.uuid("inventoryId", true, false).into_option();
    let asset = input.uuid("assetId", true, false).into_option();
    let batch = input.uuid("batchId", true, false).into_option();
    let limit = input.int("limit", 1, 500, 100);
    input.done()?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!(
        "{SELECT} WHERE t.org_id = $1 AND ($2::text IS NULL OR t.status = $2)
            AND ($6::uuid[] IS NULL OR t.to_inventory_id = ANY($6) OR t.from_inventory_id = ANY($6))
            AND ($3::uuid IS NULL OR t.to_inventory_id = $3 OR t.from_inventory_id = $3)
            AND ($4::uuid IS NULL OR t.asset_id = $4) AND ($5::uuid IS NULL OR t.batch_id = $5)
          ORDER BY t.requested_at DESC, a.asset_tag LIMIT {limit}"
    ))
    .bind(user.org_id)
    .bind(status)
    .bind(inventory)
    .bind(asset)
    .bind(batch)
    .bind(user.scope_param())
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn get(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("inventory:read")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    Ok(Json(find(&mut db, user.org_id, id).await?))
}

/// A pending transfer and the pending transfers of the components that move with it.
async fn pending_group(db: &mut Db, org: Uuid, id: Uuid) -> Result<Vec<Value>> {
    let rows = Q::new(
        "SELECT t.id, t.asset_id, t.from_inventory_id, t.requested_by, t.status FROM inventory_transfers t
          WHERE t.org_id = $1 AND (t.id = $2 OR (t.status = 'pending' AND (t.batch_id, t.via_asset_id) =
                (SELECT batch_id, asset_id FROM inventory_transfers WHERE id = $2)))
          ORDER BY t.id = $2 DESC, t.asset_id FOR UPDATE OF t",
    )
    .bind(org)
    .bind(id)
    .all(&mut *db)
    .await?;
    match rows.first() {
        None => Err(AppError::not_found("Transfer")),
        Some(first) if first["status"] != "pending" => Err(AppError::new(409, "This transfer was already decided")),
        _ => Ok(rows),
    }
}

fn note_input(body: &Value, required: bool) -> Result<Option<String>> {
    let mut input = Input::body(body);
    let rule = if required { Str::req(1, 500) } else { Str::text(500) };
    let note = input.str(if required { "reason" } else { "note" }, rule).into_option();
    input.strict()?;
    Ok(note)
}

pub async fn approve(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("transfers:approve")?;
    let id = path_uuid(&id)?;
    let note = note_input(&body, false)?;
    let mut tx = state.pool.begin().await?;
    let group = pending_group(&mut tx, user.org_id, id).await?;
    let assets: Vec<Uuid> = group.iter().filter_map(|t| uuid_of(&t["asset_id"])).collect();
    let current =
        Q::new("SELECT id, inventory_id FROM assets WHERE org_id = $1 AND id = ANY($2) ORDER BY id FOR UPDATE")
            .bind(user.org_id)
            .bind(&assets)
            .all(&mut tx)
            .await?;
    for t in &group {
        let now_in =
            current.iter().find(|a| a["id"] == t["asset_id"]).map(|a| a["inventory_id"].clone()).unwrap_or(Value::Null);
        if now_in != t["from_inventory_id"] {
            return Err(AppError::new(
                409,
                "The item was moved after this was requested. Reject it and request a new transfer.",
            ));
        }
    }
    for t in &group {
        let tid = uuid_of(&t["id"]).unwrap();
        Q::new(
            "UPDATE inventory_transfers SET status = 'completed', decided_by = $2, decided_at = now(), decision_note = $3,
                    completed_at = now() WHERE id = $1",
        )
        .bind(tid)
        .bind(user.id)
        .bind(&note)
        .run(&mut tx)
        .await?;
        apply(&mut tx, user.org_id, user.id, tid).await?;
    }
    let out = find(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    crate::events::transfer_decided(&state, user.org_id, id).await;
    Ok(Json(out))
}

async fn close(state: &AppState, user: &User, id: Uuid, status: &str, note: Option<String>) -> Result<Value> {
    let mut tx = state.pool.begin().await?;
    let group = pending_group(&mut tx, user.org_id, id).await?;
    for t in &group {
        Q::new("UPDATE inventory_transfers SET status = $2, decided_by = $3, decided_at = now(), decision_note = $4 WHERE id = $1")
            .bind(uuid_of(&t["id"]))
            .bind(status)
            .bind(user.id)
            .bind(&note)
            .run(&mut tx)
            .await?;
    }
    let out = find(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok(out)
}

pub async fn reject(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("transfers:approve")?;
    let id = path_uuid(&id)?;
    let reason = note_input(&body, true)?;
    let out = close(&state, &user, id, "rejected", reason).await?;
    crate::events::transfer_decided(&state, user.org_id, id).await;
    Ok(Json(out))
}

/// The person who asked (or an approver) withdraws a pending request.
pub async fn cancel(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("inventory:transfer")?;
    let id = path_uuid(&id)?;
    let note = note_input(&body, false)?;
    let mut db = state.pool.acquire().await?;
    let t = find(&mut db, user.org_id, id).await?;
    drop(db);
    if uuid_of(&t["requestedById"]) != Some(user.id) && user.require("transfers:approve").is_err() {
        return Err(AppError::new(403, "Only the person who asked, or a manager, can cancel this request"));
    }
    Ok(Json(close(&state, &user, id, "cancelled", note).await?))
}
