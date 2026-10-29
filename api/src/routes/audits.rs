use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::User;
use crate::db::{uuid_of, Db, Q};
use crate::error::{AppError, Result};
use crate::routes::assets::{find_by_code, record_event};
use crate::validate::{path_uuid, Input, Opt, Str};
use crate::web::Body;
use crate::AppState;

const PERIODS: &[&str] = &["weekly", "monthly", "quarterly", "annual", "manual"];
/// soft: a quick check that closes without review; full: a whole hub or scope; spot: chosen items;
/// transfer: an inventory after items moved in or out; return: returned equipment.
const TYPES: &[&str] = &["soft", "full", "spot", "transfer", "return"];

const RUN_SELECT: &str = r#"
  SELECT r.id, r.period, r.name, r.status, r.building_id AS "buildingId", b.name AS "buildingName",
         r.location_id AS "locationId", l.name AS "locationName", r.due_at AS "dueAt",
         r.started_at AS "startedAt", su.name AS "startedBy", r.closed_at AS "closedAt", cu.name AS "closedBy",
         r.summary, r.scheduled, r.audit_type AS "auditType", r.requires_review AS "requiresReview",
         r.inventory_id AS "inventoryId", inv.name AS "inventoryName",
         r.submitted_at AS "submittedAt", sb.name AS "submittedBy", r.reviewed_at AS "reviewedAt", rv.name AS "reviewedBy",
         r.review_note AS "reviewNote", r.status IN ('closed', 'approved') AS locked,
         (SELECT json_build_object(
            'expected', count(*) FILTER (WHERE i.expected),
            'verified', count(*) FILTER (WHERE i.result = 'verified' AND i.expected),
            'wrongLocation', count(*) FILTER (WHERE i.result = 'wrong_location'),
            'missing', count(*) FILTER (WHERE i.result = 'missing'),
            'pending', count(*) FILTER (WHERE i.result = 'pending'),
            'unexpected', count(*) FILTER (WHERE NOT i.expected),
            'damaged', count(*) FILTER (WHERE i.damaged),
            'duplicateScans', count(*) FILTER (WHERE i.scan_count > 1))
            FROM audit_items i WHERE i.run_id = r.id) AS counts
    FROM audit_runs r
    LEFT JOIN buildings b ON b.id = r.building_id
    LEFT JOIN locations l ON l.id = r.location_id
    LEFT JOIN users su ON su.id = r.started_by
    LEFT JOIN users cu ON cu.id = r.closed_by
    LEFT JOIN users sb ON sb.id = r.submitted_by
    LEFT JOIN users rv ON rv.id = r.reviewed_by
    LEFT JOIN inventories inv ON inv.id = r.inventory_id"#;

const ITEM_SELECT: &str = r#"
  SELECT i.asset_id AS "assetId", a.asset_tag AS "assetTag", a.name, a.type, a.status AS "assetStatus",
         i.expected, i.result, i.method, i.note, i.damaged, i.scan_count AS "scanCount", i.checked_at AS "checkedAt", u.name AS "checkedBy",
         el.name AS "expectedLocationName", eb.name AS "expectedBuildingName",
         fl.name AS "foundLocationName", pe.name AS "assignedPersonName"
    FROM audit_items i
    JOIN assets a ON a.id = i.asset_id
    LEFT JOIN locations el ON el.id = i.expected_location_id
    LEFT JOIN buildings eb ON eb.id = i.expected_building_id
    LEFT JOIN locations fl ON fl.id = i.found_location_id
    LEFT JOIN people pe ON pe.id = a.assigned_person_id
    LEFT JOIN users u ON u.id = i.checked_by"#;

async fn find_run(db: &mut Db, org_id: Uuid, id: Uuid, lock: bool) -> Result<Value> {
    Q::new(format!("{RUN_SELECT} WHERE r.org_id = $1 AND r.id = $2{}", if lock { " FOR UPDATE OF r" } else { "" }))
        .bind(org_id)
        .bind(id)
        .opt(db)
        .await?
        .ok_or_else(|| AppError::not_found("Audit"))
}

async fn review(db: &mut Db, org: Uuid, run: Uuid, action: &str, actor: Uuid, note: Option<&str>) -> Result<()> {
    Q::new("INSERT INTO audit_reviews (org_id, run_id, action, actor_id, note) VALUES ($1, $2, $3, $4, $5)")
        .bind(org)
        .bind(run)
        .bind(action)
        .bind(actor)
        .bind(note)
        .run(db)
        .await?;
    Ok(())
}

async fn find_item(db: &mut Db, run_id: Uuid, asset_id: Uuid) -> Result<Value> {
    Q::new(format!("{ITEM_SELECT} WHERE i.run_id = $1 AND i.asset_id = $2")).bind(run_id).bind(asset_id).one(db).await
}

fn require_open(run: &Value) -> Result<()> {
    match run["status"].as_str() {
        Some("open") => Ok(()),
        Some("draft") => Err(AppError::new(409, "Start this audit before recording items")),
        Some("submitted") => Err(AppError::new(409, "This audit was submitted for review and can no longer change")),
        Some("rejected") => Err(AppError::new(409, "Reopen this audit to rework it")),
        _ => Err(AppError::new(409, "This audit is closed and can no longer change")),
    }
}

fn require_status(run: &Value, status: &str, message: &str) -> Result<()> {
    if run["status"] != status {
        return Err(AppError::new(409, message.to_string()));
    }
    Ok(())
}

fn default_name(period: &str, kind: &str, scope: Option<&str>) -> String {
    let date = chrono::Utc::now().format("%b %-d, %Y");
    let label = match (kind, period) {
        ("soft", _) => "Soft",
        ("spot", _) => "Spot",
        ("transfer", _) => "Transfer",
        ("return", _) => "Return",
        (_, "weekly") => "Weekly",
        (_, "monthly") => "Monthly",
        (_, "quarterly") => "Quarterly",
        (_, "annual") => "Annual",
        _ => "Full",
    };
    match scope {
        Some(s) => format!("{label} audit · {date} · {s}"),
        None => format!("{label} audit · {date}"),
    }
}

pub async fn list(
    State(state): State<AppState>,
    user: User,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    user.require("audits:read")?;
    let mut input = Input::query(&query);
    let period = input.one_of("period", PERIODS, true, false);
    let status = input.one_of("status", &["draft", "open", "submitted", "approved", "rejected", "closed"], true, false);
    let kind = input.one_of("auditType", TYPES, true, false);
    let limit = input.int("limit", 1, 100, 25);
    input.done()?;
    let mut wheres = vec!["r.org_id = $1".to_string(), "($2::uuid[] IS NULL OR r.inventory_id = ANY($2))".to_string()];
    let mut params = vec![];
    for (column, value) in [("r.period", period), ("r.status", status), ("r.audit_type", kind)] {
        if let Opt::Val(v) = value {
            params.push(v);
            wheres.push(format!("{column} = ${}", params.len() + 2));
        }
    }
    let mut q = Q::new(format!(
        "{RUN_SELECT} WHERE {} ORDER BY r.status IN ('draft', 'open', 'submitted', 'rejected') DESC, r.started_at DESC LIMIT {limit}",
        wheres.join(" AND ")
    ))
    .bind(user.org_id)
    .bind(user.scope_param());
    for p in params {
        q = q.bind(p);
    }
    let mut db = state.pool.acquire().await?;
    Ok(Json(json!({ "items": q.all(&mut db).await? })))
}

pub struct NewAudit<'a> {
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub period: &'a str,
    pub building_id: Option<Uuid>,
    pub location_id: Option<Uuid>,
    pub name: Option<String>,
    pub due_at: Option<String>,
    pub scheduled: bool,
    pub audit_type: &'a str,
    pub requires_review: bool,
    pub inventory_id: Option<Uuid>,
    pub asset_ids: Vec<Uuid>,
    pub draft: bool,
}

impl<'a> NewAudit<'a> {
    /// A whole-workspace audit for a period, as the scheduler starts them.
    pub fn periodic(org_id: Uuid, user_id: Uuid, period: &'a str) -> NewAudit<'a> {
        NewAudit {
            org_id,
            user_id,
            period,
            building_id: None,
            location_id: None,
            name: None,
            due_at: None,
            scheduled: false,
            audit_type: "full",
            requires_review: false,
            inventory_id: None,
            asset_ids: vec![],
            draft: false,
        }
    }
}

/// Opens an audit (or a draft) and snapshots the items expected in scope: chosen items (spot),
/// an inventory and everything inside it, or a building / location. Used by people and the scheduler.
pub async fn start_audit(db: &mut Db, a: NewAudit<'_>) -> Result<Uuid> {
    if a.location_id.is_some() && a.building_id.is_none() {
        return Err(AppError::new(400, "Choose the building for that location"));
    }
    let mut scope_name = None;
    if let Some(building_id) = a.building_id {
        let scope = Q::new(
            "SELECT b.name AS building, l.name AS location FROM buildings b
               LEFT JOIN locations l ON l.id = $3 AND l.building_id = b.id
              WHERE b.org_id = $1 AND b.id = $2",
        )
        .bind(a.org_id)
        .bind(building_id)
        .bind(a.location_id)
        .opt(&mut *db)
        .await?;
        let Some(scope) = scope.filter(|s| a.location_id.is_none() || !s["location"].is_null()) else {
            return Err(AppError::new(400, "Building or location not found"));
        };
        let building = scope["building"].as_str().unwrap_or("");
        scope_name = Some(match scope["location"].as_str() {
            Some(location) => format!("{building} / {location}"),
            None => building.to_string(),
        });
    }
    if let Some(inventory) = a.inventory_id {
        let name: Option<String> =
            Q::new("SELECT name FROM inventories WHERE org_id = $1 AND id = $2 AND archived_at IS NULL")
                .bind(a.org_id)
                .bind(inventory)
                .scalars(&mut *db)
                .await?
                .pop();
        scope_name = Some(name.ok_or_else(|| AppError::new(400, "Inventory not found"))?);
    }
    let name = a.name.unwrap_or_else(|| default_name(a.period, a.audit_type, scope_name.as_deref()));
    let inserted = Q::new(
        "INSERT INTO audit_runs (org_id, period, name, building_id, location_id, due_at, started_by, scheduled,
                                 audit_type, requires_review, inventory_id, status)
         VALUES ($1, $2, $3, $4, $5,
                 COALESCE($6::timestamptz, CASE $2 WHEN 'weekly' THEN now() + interval '7 days'
                                                   WHEN 'monthly' THEN date_trunc('month', now()) + interval '1 month'
                                                   WHEN 'quarterly' THEN date_trunc('quarter', now()) + interval '3 months'
                                                   WHEN 'annual' THEN date_trunc('year', now()) + interval '1 year'
                                                   ELSE now() + interval '7 days' END),
                 $7, $8, $9, $10, $11, $12)
         RETURNING id",
    )
    .bind(a.org_id)
    .bind(a.period)
    .bind(name)
    .bind(a.building_id)
    .bind(a.location_id)
    .bind(a.due_at)
    .bind(a.user_id)
    .bind(a.scheduled)
    .bind(a.audit_type)
    .bind(a.requires_review)
    .bind(a.inventory_id)
    .bind(if a.draft { "draft" } else { "open" })
    .scalar::<Uuid>(&mut *db)
    .await;
    let id = match inserted {
        Err(e) if e.constraint() == Some("audit_runs_one_open") => {
            return Err(AppError::new(
                409,
                format!("A {} audit is already open for this scope. Close it before starting another.", a.period),
            ));
        }
        other => other?,
    };
    let n = Q::new(
        "WITH RECURSIVE sub AS (
            SELECT id, 0 AS depth FROM inventories WHERE id = $5
            UNION ALL SELECT c.id, sub.depth + 1 FROM inventories c JOIN sub ON c.parent_id = sub.id WHERE sub.depth < 20)
         INSERT INTO audit_items (run_id, org_id, asset_id, expected_building_id, expected_location_id)
         SELECT $1, org_id, id, building_id, location_id FROM assets
          WHERE org_id = $2 AND status <> 'Retired' AND archived_at IS NULL
            AND ($3::uuid IS NULL OR building_id = $3) AND ($4::uuid IS NULL OR location_id = $4)
            AND ($5::uuid IS NULL OR inventory_id IN (SELECT id FROM sub))
            AND (cardinality($6::uuid[]) = 0 OR id = ANY($6))",
    )
    .bind(id)
    .bind(a.org_id)
    .bind(a.building_id)
    .bind(a.location_id)
    .bind(a.inventory_id)
    .bind(a.asset_ids.clone())
    .run(&mut *db)
    .await?;
    if !a.asset_ids.is_empty() && n as usize != a.asset_ids.len() {
        return Err(AppError::new(400, "Some chosen items were not found, are archived or are outside this scope"));
    }
    review(db, a.org_id, id, "started", a.user_id, None).await?;
    Ok(id)
}

fn is_datetime(s: &str) -> Option<&'static str> {
    chrono::DateTime::parse_from_rfc3339(s).is_err().then_some("Invalid datetime")
}

pub async fn start(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("audits:perform")?;
    let mut input = Input::body(&body);
    let kind = input.one_of("auditType", TYPES, true, false).into_option();
    // Without a type this is the original weekly / monthly audit, which needs a period.
    let period = input.one_of("period", PERIODS, kind.is_some(), false).into_option();
    let building_id = input.uuid("buildingId", true, true).into_option();
    let location_id = input.uuid("locationId", true, true).into_option();
    let inventory_id = input.uuid("inventoryId", true, true).into_option();
    let mut asset_ids = vec![];
    match input.raw_value("assetIds") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) if items.len() <= 500 => {
            for item in items {
                match item.as_str().and_then(crate::validate::parse_uuid) {
                    Some(id) if !asset_ids.contains(&id) => asset_ids.push(id),
                    Some(_) => {}
                    None => input.fail("assetIds", "Must be a list of item ids"),
                }
            }
        }
        Some(_) => input.fail("assetIds", "Must be a list of at most 500 item ids"),
    }
    let review_default = !matches!(kind.as_deref(), None | Some("soft"));
    let requires_review = input.boolean("requiresReview", false).into_option().unwrap_or(review_default);
    let draft = input.boolean_or("draft", false);
    let name = input.str("name", Str::req(1, 160).nullish()).into_option();
    let due_at = input.str("dueAt", Str { trim: false, ..Str::req(0, 100).nullish() }.check(is_datetime)).into_option();
    input.strict()?;
    let kind = kind.unwrap_or_else(|| "full".into());
    let period = period.unwrap_or_else(|| "manual".into());
    let mut tx = state.pool.begin().await?;
    let mut inventory_id = inventory_id;
    match kind.as_str() {
        "spot" if asset_ids.is_empty() && inventory_id.is_none() => {
            return Err(AppError::new(400, "Choose the items or the inventory for a spot audit"));
        }
        "transfer" if inventory_id.is_none() => {
            return Err(AppError::new(400, "Choose the inventory a transfer audit checks"));
        }
        // A return audit checks the returned-equipment inventory unless another is chosen.
        "return" if inventory_id.is_none() => {
            inventory_id = Q::new(
                "SELECT id FROM inventories WHERE org_id = $1 AND kind = 'returned' AND archived_at IS NULL ORDER BY created_at LIMIT 1",
            )
            .bind(user.org_id)
            .scalars(&mut tx)
            .await?
            .pop();
            if inventory_id.is_none() {
                return Err(AppError::new(400, "Choose the inventory a return audit checks"));
            }
        }
        _ => {}
    }
    if kind != "spot" && !asset_ids.is_empty() {
        return Err(AppError::new(400, "Only spot audits take a list of items"));
    }
    // People with access to some inventories audit only inside them.
    if user.scope.is_some() {
        if !user.sees_inventory(inventory_id) {
            return Err(AppError::new(400, "Inventory not found"));
        }
        if building_id.is_some() {
            return Err(AppError::new(403, "Your access is limited to selected inventories"));
        }
    }
    let id = start_audit(
        &mut tx,
        NewAudit {
            building_id,
            location_id,
            name,
            due_at,
            audit_type: &kind,
            requires_review,
            inventory_id,
            asset_ids,
            draft,
            ..NewAudit::periodic(user.org_id, user.id, &period)
        },
    )
    .await?;
    let run = find_run(&mut tx, user.org_id, id, false).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(run)))
}

/// A draft becomes an audit in progress.
pub async fn begin(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("audits:perform")?;
    let id = path_uuid(&id)?;
    let mut tx = state.pool.begin().await?;
    let run = find_run(&mut tx, user.org_id, id, true).await?;
    require_status(&run, "draft", "Only a draft can be started")?;
    Q::new("UPDATE audit_runs SET status = 'open', started_at = now() WHERE id = $1").bind(id).run(&mut tx).await?;
    let run = find_run(&mut tx, user.org_id, id, false).await?;
    tx.commit().await?;
    Ok(Json(run))
}

pub async fn get(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("audits:read")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let mut run = find_run(&mut db, user.org_id, id, false).await?;
    run["items"] = json!(items_of(&mut db, id).await?);
    run["reviews"] = json!(
        Q::new(
            r#"SELECT v.action, v.note, v.created_at AS "createdAt", u.name AS "actorName"
                 FROM audit_reviews v LEFT JOIN users u ON u.id = v.actor_id WHERE v.run_id = $1 ORDER BY v.id"#,
        )
        .bind(id)
        .all(&mut db)
        .await?
    );
    Ok(Json(run))
}

async fn items_of(db: &mut Db, id: Uuid) -> Result<Vec<Value>> {
    Q::new(format!(
        "{ITEM_SELECT} WHERE i.run_id = $1
          ORDER BY CASE i.result WHEN 'pending' THEN 0 WHEN 'wrong_location' THEN 1 WHEN 'missing' THEN 2 ELSE 3 END,
                   a.asset_tag"
    ))
    .bind(id)
    .all(db)
    .await
}

/// The audit's items as a CSV report.
pub async fn export(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
) -> Result<axum::response::Response> {
    user.require("audits:read")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let run = find_run(&mut db, user.org_id, id, false).await?;
    let items = items_of(&mut db, id).await?;
    let columns = [
        ("assetTag", "Asset tag"),
        ("name", "Item"),
        ("type", "Category"),
        ("result", "Result"),
        ("expected", "Expected"),
        ("damaged", "Damaged"),
        ("scanCount", "Scans"),
        ("expectedBuildingName", "Expected building"),
        ("expectedLocationName", "Expected location"),
        ("foundLocationName", "Found at"),
        ("method", "Method"),
        ("checkedBy", "Checked by"),
        ("checkedAt", "Checked at"),
        ("note", "Note"),
    ];
    let slug: String =
        run["name"].as_str().unwrap_or("audit").chars().filter(|c| c.is_ascii_alphanumeric()).take(40).collect();
    Ok(crate::routes::assets::csv_response(&columns, &items, &format!("audit-{slug}.csv")))
}

pub async fn scan(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("audits:perform")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let code = input.req("code", Str::req(1, 500));
    let location_id = input.uuid("locationId", true, true).into_option();
    input.strict()?;
    let org = user.org_id;
    let mut tx = state.pool.begin().await?;
    let run = find_run(&mut tx, org, id, true).await?;
    require_open(&run)?;
    let asset = find_by_code(&mut tx, org, &code).await?;
    if !user.sees_inventory(uuid_of(&asset["inventoryId"])) {
        return Err(AppError::new(404, "No asset matches that code"));
    }
    let found_location = location_id.or_else(|| uuid_of(&run["locationId"]));
    if let Some(location_id) = location_id {
        let exists = Q::new("SELECT 1 FROM locations WHERE org_id = $1 AND id = $2")
            .bind(org)
            .bind(location_id)
            .run(&mut tx)
            .await?;
        if exists == 0 {
            return Err(AppError::new(400, "Location not found"));
        }
    }
    let asset_id = uuid_of(&asset["id"]).unwrap();
    // An asset counts as misplaced when the auditor says where they are and it is not where it was
    // expected. Scanning the same item again is counted, so duplicates show in the report.
    Q::new(
        "INSERT INTO audit_items (run_id, org_id, asset_id, expected, result, found_location_id, method, checked_by, checked_at, scan_count)
         VALUES ($1, $2, $3, false, 'verified', $4, 'scan', $5, now(), 1)
         ON CONFLICT (run_id, asset_id) DO UPDATE SET
           result = CASE WHEN EXCLUDED.found_location_id IS NOT NULL AND audit_items.expected_location_id IS NOT NULL
                              AND EXCLUDED.found_location_id <> audit_items.expected_location_id
                         THEN 'wrong_location' ELSE 'verified' END,
           found_location_id = EXCLUDED.found_location_id, method = 'scan',
           checked_by = EXCLUDED.checked_by, checked_at = now(), scan_count = audit_items.scan_count + 1",
    )
    .bind(id)
    .bind(org)
    .bind(asset_id)
    .bind(found_location)
    .bind(user.id)
    .run(&mut tx)
    .await?;
    let mut item = find_item(&mut tx, id, asset_id).await?;
    item["duplicate"] = json!(item["scanCount"].as_i64().unwrap_or(0) > 1);
    tx.commit().await?;
    Ok(Json(item))
}

pub async fn mark_item(
    State(state): State<AppState>,
    user: User,
    Path((id, asset_id)): Path<(String, String)>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("audits:perform")?;
    let id = path_uuid(&id)?;
    let asset_id = path_uuid(&asset_id)?;
    let mut input = Input::body(&body);
    let result =
        input.one_of("result", &["verified", "missing", "pending"], false, false).into_option().unwrap_or_default();
    let note = input.str("note", Str::text(500)).into_option();
    let damaged = input.boolean("damaged", false).into_option();
    input.strict()?;
    if damaged == Some(true) && note.is_none() {
        return Err(AppError::new(400, "Describe the damage in a note"));
    }
    let mut tx = state.pool.begin().await?;
    require_open(&find_run(&mut tx, user.org_id, id, true).await?)?;
    let pending = result == "pending";
    let updated = Q::new(
        "UPDATE audit_items SET result = $3, method = $4, note = COALESCE($5, note),
                checked_by = $6, checked_at = CASE WHEN $6::uuid IS NULL THEN NULL ELSE now() END, found_location_id = NULL,
                damaged = COALESCE($7, damaged)
          WHERE run_id = $1 AND asset_id = $2",
    )
    .bind(id)
    .bind(asset_id)
    .bind(&result)
    .bind((!pending).then_some("manual"))
    .bind(note)
    .bind((!pending).then_some(user.id))
    .bind(damaged)
    .run(&mut tx)
    .await?;
    if updated == 0 {
        return Err(AppError::not_found("Audit item"));
    }
    let item = find_item(&mut tx, id, asset_id).await?;
    tx.commit().await?;
    Ok(Json(item))
}

/// Records anything still pending as missing (the audit is over).
async fn mark_rest_missing(db: &mut Db, id: Uuid, user: Uuid) -> Result<()> {
    Q::new("UPDATE audit_items SET result = 'missing', checked_by = $2, checked_at = now() WHERE run_id = $1 AND result = 'pending'")
        .bind(id)
        .bind(user)
        .run(db)
        .await?;
    Ok(())
}

/// Finishes an audit: every item's result goes on its history, and the run is locked with its summary.
async fn finish(db: &mut Db, user: &User, current: &Value, status: &str) -> Result<()> {
    let org = user.org_id;
    let id = uuid_of(&current["id"]).unwrap();
    let items = Q::new("SELECT asset_id, expected, result, damaged FROM audit_items WHERE run_id = $1")
        .bind(id)
        .all(&mut *db)
        .await?;
    for item in items {
        let audit = json!({
            "id": id, "name": current["name"], "period": current["period"], "type": current["auditType"],
            "result": item["result"], "expected": item["expected"], "damaged": item["damaged"],
        });
        record_event(
            &mut *db,
            org,
            uuid_of(&item["asset_id"]).unwrap(),
            user.id,
            "audited",
            json!({ "audit": { "from": null, "to": audit } }),
        )
        .await?;
    }
    let counts = find_run(&mut *db, org, id, false).await?["counts"].clone();
    Q::new("UPDATE audit_runs SET status = $4, closed_by = $2, closed_at = now(), summary = $3 WHERE id = $1")
        .bind(id)
        .bind(user.id)
        .bind(counts)
        .bind(status)
        .run(db)
        .await?;
    Ok(())
}

/// Closes an audit that needs no review.
pub async fn close(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("audits:perform")?;
    let id = path_uuid(&id)?;
    let org = user.org_id;
    let mut tx = state.pool.begin().await?;
    let current = find_run(&mut tx, org, id, true).await?;
    require_open(&current)?;
    if current["requiresReview"] == true {
        return Err(AppError::new(409, "This audit needs a manager's review. Submit it instead."));
    }
    mark_rest_missing(&mut tx, id, user.id).await?;
    finish(&mut tx, &user, &current, "closed").await?;
    review(&mut tx, org, id, "closed", user.id, None).await?;
    let run = find_run(&mut tx, org, id, false).await?;
    tx.commit().await?;
    Ok(Json(run))
}

fn note_of(body: &Value, required: bool) -> Result<Option<String>> {
    let mut input = Input::body(body);
    let rule = if required { Str::req(1, 1000) } else { Str::text(1000) };
    let note = input.str("note", rule).into_option();
    input.strict()?;
    Ok(note)
}

/// Sends a finished audit for review; whatever was not found is recorded as missing.
pub async fn submit(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("audits:perform")?;
    let id = path_uuid(&id)?;
    let note = note_of(&body, false)?;
    let mut tx = state.pool.begin().await?;
    let current = find_run(&mut tx, user.org_id, id, true).await?;
    require_open(&current)?;
    if current["requiresReview"] != true {
        return Err(AppError::new(409, "This audit needs no review. Close it instead."));
    }
    mark_rest_missing(&mut tx, id, user.id).await?;
    Q::new("UPDATE audit_runs SET status = 'submitted', submitted_by = $2, submitted_at = now() WHERE id = $1")
        .bind(id)
        .bind(user.id)
        .run(&mut tx)
        .await?;
    review(&mut tx, user.org_id, id, "submitted", user.id, note.as_deref()).await?;
    let run = find_run(&mut tx, user.org_id, id, false).await?;
    tx.commit().await?;
    crate::events::audit_submitted(&state, user.org_id, id).await;
    Ok(Json(run))
}

/// A manager or admin who neither started nor submitted the audit approves (and locks) or rejects it.
async fn decide(state: AppState, user: User, id: String, body: Value, approve: bool) -> Result<Json<Value>> {
    user.require("audits:approve")?;
    let id = path_uuid(&id)?;
    let note = note_of(&body, !approve)?;
    let mut tx = state.pool.begin().await?;
    let current = find_run(&mut tx, user.org_id, id, true).await?;
    require_status(&current, "submitted", "Only a submitted audit can be reviewed")?;
    let authors: Vec<Option<Uuid>> =
        Q::new("SELECT unnest(ARRAY[started_by, submitted_by]) FROM audit_runs WHERE id = $1")
            .bind(id)
            .scalars(&mut tx)
            .await?;
    if authors.contains(&Some(user.id)) {
        return Err(AppError::new(403, "You cannot review an audit you started or submitted"));
    }
    Q::new("UPDATE audit_runs SET reviewed_by = $2, reviewed_at = now(), review_note = $3 WHERE id = $1")
        .bind(id)
        .bind(user.id)
        .bind(&note)
        .run(&mut tx)
        .await?;
    if approve {
        finish(&mut tx, &user, &current, "approved").await?;
    } else {
        Q::new("UPDATE audit_runs SET status = 'rejected' WHERE id = $1").bind(id).run(&mut tx).await?;
    }
    review(&mut tx, user.org_id, id, if approve { "approved" } else { "rejected" }, user.id, note.as_deref()).await?;
    let run = find_run(&mut tx, user.org_id, id, false).await?;
    tx.commit().await?;
    crate::events::audit_decided(&state, user.org_id, id).await;
    Ok(Json(run))
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

/// A rejected audit goes back to in progress for rework; its review history stays.
pub async fn reopen(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("audits:perform")?;
    let id = path_uuid(&id)?;
    let note = note_of(&body, false)?;
    let mut tx = state.pool.begin().await?;
    let current = find_run(&mut tx, user.org_id, id, true).await?;
    require_status(&current, "rejected", "Only a rejected audit can be reopened")?;
    Q::new(
        "UPDATE audit_runs SET status = 'open', submitted_by = NULL, submitted_at = NULL, reviewed_by = NULL, reviewed_at = NULL
          WHERE id = $1",
    )
    .bind(id)
    .run(&mut tx)
    .await?;
    review(&mut tx, user.org_id, id, "reopened", user.id, note.as_deref()).await?;
    let run = find_run(&mut tx, user.org_id, id, false).await?;
    tx.commit().await?;
    Ok(Json(run))
}
