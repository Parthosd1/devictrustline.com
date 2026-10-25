use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{can, User};
use crate::db::{uuid_of, Assign, Db, Param, Q};
use crate::error::{AppError, Result};
use crate::routes::assets::record_event;
use crate::validate::{path_uuid, Input, Opt, Str, DATE};
use crate::web::Body;
use crate::AppState;

const PRIORITIES: &[&str] = &["low", "normal", "high", "urgent"];
const ACTIVE: &[&str] = &["open", "in_progress", "waiting_parts"];

fn status_label(status: &str) -> &'static str {
    match status {
        "open" => "Open",
        "in_progress" => "In progress",
        "waiting_parts" => "Waiting on parts",
        "resolved" => "Resolved",
        _ => "Cancelled",
    }
}

const SELECT: &str = r#"
  SELECT w.id, w.number, w.title, w.description, w.priority, w.status, w.vendor, w.cost::float AS cost,
         w.due_date AS "dueDate", w.resolution, w.opened_at AS "openedAt", w.closed_at AS "closedAt",
         w.updated_at AS "updatedAt", ou.name AS "openedBy",
         w.assignee_id AS "assigneeId", au.name AS "assigneeName",
         w.asset_id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName", a.type AS "assetType",
         a.status AS "assetStatus", b.name AS "buildingName", l.name AS "locationName",
         (w.status IN ('open', 'in_progress', 'waiting_parts') AND w.due_date < current_date) AS overdue
    FROM work_orders w
    JOIN assets a ON a.id = w.asset_id
    LEFT JOIN buildings b ON b.id = a.building_id
    LEFT JOIN locations l ON l.id = a.location_id
    LEFT JOIN users ou ON ou.id = w.opened_by
    LEFT JOIN users au ON au.id = w.assignee_id"#;

async fn find_work_order(db: &mut Db, org_id: Uuid, id: Uuid, lock: bool) -> Result<Value> {
    Q::new(format!("{SELECT} WHERE w.org_id = $1 AND w.id = $2{}", if lock { " FOR UPDATE OF w" } else { "" }))
        .bind(org_id)
        .bind(id)
        .opt(db)
        .await?
        .ok_or_else(|| AppError::not_found("Work order"))
}

async fn check_assignee(db: &mut Db, org_id: Uuid, assignee: Option<Uuid>) -> Result<()> {
    let Some(assignee) = assignee else { return Ok(()) };
    let u = Q::new("SELECT role, is_active FROM users WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(assignee)
        .opt(db)
        .await?;
    let ok = u.is_some_and(|u| u["is_active"] == true && can(u["role"].as_str().unwrap_or(""), "maintenance:write"));
    if !ok {
        return Err(AppError::new(400, "Assign work orders to an active admin, manager or technician"));
    }
    Ok(())
}

async fn add_note(db: &mut Db, org_id: Uuid, id: Uuid, author: Uuid, kind: &str, body: &str) -> Result<()> {
    Q::new("INSERT INTO work_order_notes (org_id, work_order_id, author_id, kind, body) VALUES ($1, $2, $3, $4, $5)")
        .bind(org_id)
        .bind(id)
        .bind(author)
        .bind(kind)
        .bind(body)
        .run(db)
        .await?;
    Ok(())
}

async fn set_asset_status(
    db: &mut Db,
    org_id: Uuid,
    asset_id: Uuid,
    actor: Uuid,
    status: &str,
    work_order: &str,
) -> Result<()> {
    let current: String = Q::new("SELECT status FROM assets WHERE org_id = $1 AND id = $2 FOR UPDATE")
        .bind(org_id)
        .bind(asset_id)
        .scalar(&mut *db)
        .await?;
    if current == status {
        return Ok(());
    }
    Q::new("UPDATE assets SET status = $3, updated_at = now() WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(asset_id)
        .bind(status)
        .run(&mut *db)
        .await?;
    let changes =
        json!({ "status": { "from": current, "to": status }, "workOrder": { "from": null, "to": work_order } });
    record_event(db, org_id, asset_id, actor, "status_changed", changes).await
}

pub async fn list(
    State(state): State<AppState>,
    user: User,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    user.require("maintenance:read")?;
    let mut input = Input::query(&query);
    let statuses = ["open", "in_progress", "waiting_parts", "resolved", "cancelled", "active", "closed"];
    let status = input.one_of("status", &statuses, true, false);
    let asset = input.uuid("assetId", true, false);
    let assignee = input.uuid("assigneeId", true, false);
    let limit = input.int("limit", 1, 500, 200);
    input.done()?;

    let mut wheres = vec!["w.org_id = $1".to_string()];
    let mut status_param = None;
    match status.value().map(String::as_str) {
        Some("active") => wheres.push("w.status IN ('open', 'in_progress', 'waiting_parts')".into()),
        Some("closed") => wheres.push("w.status IN ('resolved', 'cancelled')".into()),
        Some(s) => {
            wheres.push("w.status = $2".into());
            status_param = Some(s.to_string());
        }
        None => {}
    }
    let mut uuids = vec![];
    for (column, value) in [("w.asset_id", asset), ("w.assignee_id", assignee)] {
        if let Opt::Val(u) = value {
            uuids.push(u);
            wheres.push(format!("{column} = ${}", 1 + status_param.is_some() as usize + uuids.len()));
        }
    }
    let mut q = Q::new(format!(
        "{SELECT} WHERE {}
          ORDER BY (w.status IN ('resolved', 'cancelled')), array_position(ARRAY['urgent','high','normal','low'], w.priority),
                   w.opened_at DESC LIMIT {limit}",
        wheres.join(" AND ")
    ))
    .bind(user.org_id);
    if let Some(s) = status_param {
        q = q.bind(s);
    }
    for u in uuids {
        q = q.bind(u);
    }
    let mut db = state.pool.acquire().await?;
    Ok(Json(json!({ "items": q.all(&mut db).await? })))
}

/// People who can be given work orders.
pub async fn assignees(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("maintenance:read")?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(
        "SELECT id, name, role FROM users WHERE org_id = $1 AND is_active AND role IN ('admin', 'manager', 'technician') ORDER BY name",
    )
    .bind(user.org_id)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn create(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("maintenance:write")?;
    let mut input = Input::body(&body);
    let asset_id = input.uuid("assetId", false, false).into_option();
    let title = input.req("title", Str::req(1, 160));
    let description = input.str("description", Str::text(4000)).into_option();
    let priority = input.one_of_or("priority", PRIORITIES, "normal");
    let assignee = input.uuid("assigneeId", true, true).into_option();
    let vendor = input.str("vendor", Str::text(160)).into_option();
    let due_date = input.str("dueDate", DATE).into_option();
    // Puts the asset into Maintenance status while the work order is open.
    let to_maintenance = input.boolean_or("setAssetToMaintenance", true);
    input.strict()?;
    let asset_id = asset_id.unwrap();
    let org = user.org_id;

    let mut tx = state.pool.begin().await?;
    let asset = Q::new("SELECT status FROM assets WHERE org_id = $1 AND id = $2")
        .bind(org)
        .bind(asset_id)
        .opt(&mut tx)
        .await?
        .ok_or_else(|| AppError::new(400, "Asset not found"))?;
    if asset["status"] == "Retired" {
        return Err(AppError::new(409, "Retired assets cannot get work orders"));
    }
    check_assignee(&mut tx, org, assignee).await?;
    let n: i32 = Q::new(
        "UPDATE organizations SET next_work_order_number = next_work_order_number + 1 WHERE id = $1
         RETURNING next_work_order_number - 1",
    )
    .bind(org)
    .scalar(&mut tx)
    .await?;
    let number = format!("WO-{n}");
    let id: Uuid = Q::new(
        "INSERT INTO work_orders (org_id, number, asset_id, title, description, priority, assignee_id, vendor, due_date, opened_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::date, $10) RETURNING id",
    )
    .bind(org)
    .bind(&number)
    .bind(asset_id)
    .bind(&title)
    .bind(description)
    .bind(priority)
    .bind(assignee)
    .bind(vendor)
    .bind(due_date)
    .bind(user.id)
    .scalar(&mut tx)
    .await?;
    add_note(&mut tx, org, id, user.id, "status", "Opened").await?;
    let changes = json!({ "workOrder": { "from": null, "to": { "id": id, "number": number, "title": title } } });
    record_event(&mut tx, org, asset_id, user.id, "maintenance_opened", changes).await?;
    if to_maintenance {
        set_asset_status(&mut tx, org, asset_id, user.id, "Maintenance", &number).await?;
    }
    let wo = find_work_order(&mut tx, org, id, false).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(wo)))
}

pub async fn get(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("maintenance:read")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let mut wo = find_work_order(&mut db, user.org_id, id, false).await?;
    let notes = Q::new(
        r#"SELECT n.id, n.kind, n.body, n.created_at AS "createdAt", u.name AS author
             FROM work_order_notes n LEFT JOIN users u ON u.id = n.author_id
            WHERE n.work_order_id = $1 ORDER BY n.id"#,
    )
    .bind(id)
    .all(&mut db)
    .await?;
    wo["notes"] = json!(notes);
    Ok(Json(wo))
}

pub async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("maintenance:write")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let title = input.str("title", Str::req(1, 160).optional());
    let description = input.str("description", Str::text(4000));
    let priority = input.one_of("priority", PRIORITIES, true, false);
    let status = input.one_of("status", ACTIVE, true, false);
    let assignee = input.uuid("assigneeId", true, true);
    let vendor = input.str("vendor", Str::text(160));
    let cost = input.money("cost");
    let due_date = input.str("dueDate", DATE);
    input.strict()?;
    let org = user.org_id;

    let mut tx = state.pool.begin().await?;
    let current = find_work_order(&mut tx, org, id, true).await?;
    if !ACTIVE.contains(&current["status"].as_str().unwrap_or("")) {
        return Err(AppError::new(409, "This work order is closed"));
    }
    if let Opt::Val(a) = assignee {
        check_assignee(&mut tx, org, Some(a)).await?;
    }
    let text = |o: &Opt<String>| Param::Text(o.value().cloned());
    let mut assign = Assign::new(3);
    let fields: [(&str, &str, Option<Param>); 8] = [
        ("title", "", title.is_present().then(|| text(&title))),
        ("description", "", description.is_present().then(|| text(&description))),
        ("priority", "", priority.is_present().then(|| text(&priority))),
        ("status", "", status.is_present().then(|| text(&status))),
        ("assignee_id", "::uuid", assignee.is_present().then(|| Param::Text(assignee.value().map(Uuid::to_string)))),
        ("vendor", "", vendor.is_present().then(|| text(&vendor))),
        ("cost", "::numeric", cost.is_present().then(|| Param::Text(cost.value().map(f64::to_string)))),
        ("due_date", "::date", due_date.is_present().then(|| text(&due_date))),
    ];
    for (column, cast, value) in fields {
        if let Some(value) = value {
            assign.set(column, cast, value);
        }
    }
    if !assign.is_empty() {
        let sql = format!("UPDATE work_orders SET {}, updated_at = now() WHERE org_id = $1 AND id = $2", assign.sql());
        assign.bind_to(Q::new(sql).bind(org).bind(id)).run(&mut tx).await?;
    }
    if let Opt::Val(s) = &status {
        if current["status"] != s.as_str() {
            add_note(&mut tx, org, id, user.id, "status", &format!("Status: {}", status_label(s))).await?;
        }
    }
    if assignee.is_present() && assignee.value().copied() != uuid_of(&current["assigneeId"]) {
        let body = match assignee.value() {
            Some(a) => {
                let name: String = Q::new("SELECT name FROM users WHERE id = $1").bind(*a).scalar(&mut tx).await?;
                format!("Assigned to {name}")
            }
            None => "Unassigned".into(),
        };
        add_note(&mut tx, org, id, user.id, "status", &body).await?;
    }
    let wo = find_work_order(&mut tx, org, id, false).await?;
    tx.commit().await?;
    Ok(Json(wo))
}

pub async fn add_note_route(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<(StatusCode, Json<Value>)> {
    user.require("maintenance:write")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let text = input.req("body", Str::req(1, 4000));
    input.strict()?;
    let mut db = state.pool.acquire().await?;
    find_work_order(&mut db, user.org_id, id, false).await?;
    add_note(&mut db, user.org_id, id, user.id, "note", &text).await?;
    Ok((StatusCode::CREATED, Json(json!({ "ok": true }))))
}

async fn close(
    state: &AppState,
    user: &User,
    id: Uuid,
    status: &str,
    text: &str,
    cost: Option<f64>,
    asset_status: Option<String>,
) -> Result<Value> {
    let org = user.org_id;
    let mut tx = state.pool.begin().await?;
    let current = find_work_order(&mut tx, org, id, true).await?;
    if !ACTIVE.contains(&current["status"].as_str().unwrap_or("")) {
        return Err(AppError::new(409, "This work order is already closed"));
    }
    Q::new(
        "UPDATE work_orders SET status = $3, resolution = $4, cost = COALESCE($5::numeric, cost), closed_at = now(), updated_at = now()
          WHERE org_id = $1 AND id = $2",
    )
    .bind(org)
    .bind(id)
    .bind(status)
    .bind(text)
    .bind(cost.map(|c| c.to_string()))
    .run(&mut tx)
    .await?;
    add_note(&mut tx, org, id, user.id, "status", &format!("{}: {text}", status_label(status))).await?;
    let asset_id = uuid_of(&current["assetId"]).unwrap();
    let number = current["number"].as_str().unwrap_or("").to_string();
    let changes = json!({ "workOrder": { "from": null, "to": {
        "id": id, "number": number, "title": current["title"], "status": status, "resolution": text,
    } } });
    record_event(&mut tx, org, asset_id, user.id, "maintenance_closed", changes).await?;
    // Release the asset from Maintenance once no other work order is still active on it.
    let others: i32 = Q::new(
        "SELECT count(*)::int FROM work_orders WHERE asset_id = $1 AND id <> $2 AND status IN ('open', 'in_progress', 'waiting_parts')",
    )
    .bind(asset_id)
    .bind(id)
    .scalar(&mut tx)
    .await?;
    if others == 0 && (current["assetStatus"] == "Maintenance" || asset_status.is_some()) {
        let holder: Option<Uuid> =
            Q::new("SELECT assigned_person_id FROM assets WHERE id = $1").bind(asset_id).scalar(&mut tx).await?;
        let next = asset_status.unwrap_or_else(|| if holder.is_some() { "Deployed" } else { "Available" }.to_string());
        set_asset_status(&mut tx, org, asset_id, user.id, &next, &number).await?;
    }
    let wo = find_work_order(&mut tx, org, id, false).await?;
    tx.commit().await?;
    Ok(wo)
}

pub async fn resolve(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("maintenance:write")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let resolution = input.req("resolution", Str::req(1, 4000));
    let cost = input.money("cost").into_option();
    // What the asset becomes once fixed; by default Deployed if someone holds it, otherwise Available.
    let asset_status = input.one_of("assetStatus", &["Available", "Deployed", "Retired"], true, true).into_option();
    input.strict()?;
    Ok(Json(close(&state, &user, id, "resolved", &resolution, cost, asset_status).await?))
}

pub async fn cancel(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("maintenance:write")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let reason = input.req("reason", Str::req(1, 4000));
    input.strict()?;
    Ok(Json(close(&state, &user, id, "cancelled", &reason, None, None).await?))
}
