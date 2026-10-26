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

const PERIODS: &[&str] = &["weekly", "monthly"];

const RUN_SELECT: &str = r#"
  SELECT r.id, r.period, r.name, r.status, r.building_id AS "buildingId", b.name AS "buildingName",
         r.location_id AS "locationId", l.name AS "locationName", r.due_at AS "dueAt",
         r.started_at AS "startedAt", su.name AS "startedBy", r.closed_at AS "closedAt", cu.name AS "closedBy",
         r.summary, r.scheduled,
         (SELECT json_build_object(
            'expected', count(*) FILTER (WHERE i.expected),
            'verified', count(*) FILTER (WHERE i.result = 'verified' AND i.expected),
            'wrongLocation', count(*) FILTER (WHERE i.result = 'wrong_location'),
            'missing', count(*) FILTER (WHERE i.result = 'missing'),
            'pending', count(*) FILTER (WHERE i.result = 'pending'),
            'unexpected', count(*) FILTER (WHERE NOT i.expected))
            FROM audit_items i WHERE i.run_id = r.id) AS counts
    FROM audit_runs r
    LEFT JOIN buildings b ON b.id = r.building_id
    LEFT JOIN locations l ON l.id = r.location_id
    LEFT JOIN users su ON su.id = r.started_by
    LEFT JOIN users cu ON cu.id = r.closed_by"#;

const ITEM_SELECT: &str = r#"
  SELECT i.asset_id AS "assetId", a.asset_tag AS "assetTag", a.name, a.type, a.status AS "assetStatus",
         i.expected, i.result, i.method, i.note, i.checked_at AS "checkedAt", u.name AS "checkedBy",
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

async fn find_item(db: &mut Db, run_id: Uuid, asset_id: Uuid) -> Result<Value> {
    Q::new(format!("{ITEM_SELECT} WHERE i.run_id = $1 AND i.asset_id = $2")).bind(run_id).bind(asset_id).one(db).await
}

fn require_open(run: &Value) -> Result<()> {
    if run["status"] != "open" {
        return Err(AppError::new(409, "This audit is closed and can no longer change"));
    }
    Ok(())
}

fn default_name(period: &str, scope: Option<&str>) -> String {
    let date = chrono::Utc::now().format("%b %-d, %Y");
    let label = if period == "weekly" { "Weekly" } else { "Monthly" };
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
    let status = input.one_of("status", &["open", "closed"], true, false);
    let limit = input.int("limit", 1, 100, 25);
    input.done()?;
    let mut wheres = vec!["r.org_id = $1".to_string()];
    let mut params = vec![];
    for (column, value) in [("r.period", period), ("r.status", status)] {
        if let Opt::Val(v) = value {
            params.push(v);
            wheres.push(format!("{column} = ${}", params.len() + 1));
        }
    }
    let mut q = Q::new(format!(
        "{RUN_SELECT} WHERE {} ORDER BY r.status = 'open' DESC, r.started_at DESC LIMIT {limit}",
        wheres.join(" AND ")
    ))
    .bind(user.org_id);
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
}

/// Opens an audit and snapshots every active asset in scope. Used by people and by the scheduler.
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
    let name = a.name.unwrap_or_else(|| default_name(a.period, scope_name.as_deref()));
    let inserted = Q::new(
        "INSERT INTO audit_runs (org_id, period, name, building_id, location_id, due_at, started_by, scheduled)
         VALUES ($1, $2, $3, $4, $5,
                 COALESCE($6::timestamptz, CASE WHEN $2 = 'weekly' THEN now() + interval '7 days'
                                                ELSE date_trunc('month', now()) + interval '1 month' END),
                 $7, $8)
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
    Q::new(
        "INSERT INTO audit_items (run_id, org_id, asset_id, expected_building_id, expected_location_id)
         SELECT $1, org_id, id, building_id, location_id FROM assets
          WHERE org_id = $2 AND status <> 'Retired'
            AND ($3::uuid IS NULL OR building_id = $3) AND ($4::uuid IS NULL OR location_id = $4)",
    )
    .bind(id)
    .bind(a.org_id)
    .bind(a.building_id)
    .bind(a.location_id)
    .run(db)
    .await?;
    Ok(id)
}

fn is_datetime(s: &str) -> Option<&'static str> {
    chrono::DateTime::parse_from_rfc3339(s).is_err().then_some("Invalid datetime")
}

pub async fn start(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("audits:perform")?;
    let mut input = Input::body(&body);
    let period = input.one_of("period", PERIODS, false, false);
    let building_id = input.uuid("buildingId", true, true).into_option();
    let location_id = input.uuid("locationId", true, true).into_option();
    let name = input.str("name", Str::req(1, 160).nullish()).into_option();
    let due_at = input.str("dueAt", Str { trim: false, ..Str::req(0, 100).nullish() }.check(is_datetime)).into_option();
    input.strict()?;
    let period = period.into_option().unwrap();
    let mut tx = state.pool.begin().await?;
    let id = start_audit(
        &mut tx,
        NewAudit {
            org_id: user.org_id,
            user_id: user.id,
            period: &period,
            building_id,
            location_id,
            name,
            due_at,
            scheduled: false,
        },
    )
    .await?;
    let run = find_run(&mut tx, user.org_id, id, false).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(run)))
}

pub async fn get(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("audits:read")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let mut run = find_run(&mut db, user.org_id, id, false).await?;
    let items = Q::new(format!(
        "{ITEM_SELECT} WHERE i.run_id = $1
          ORDER BY CASE i.result WHEN 'pending' THEN 0 WHEN 'wrong_location' THEN 1 WHEN 'missing' THEN 2 ELSE 3 END,
                   a.asset_tag"
    ))
    .bind(id)
    .all(&mut db)
    .await?;
    run["items"] = json!(items);
    Ok(Json(run))
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
    // An asset counts as misplaced when the auditor says where they are and it is not where it was expected.
    Q::new(
        "INSERT INTO audit_items (run_id, org_id, asset_id, expected, result, found_location_id, method, checked_by, checked_at)
         VALUES ($1, $2, $3, false, 'verified', $4, 'scan', $5, now())
         ON CONFLICT (run_id, asset_id) DO UPDATE SET
           result = CASE WHEN EXCLUDED.found_location_id IS NOT NULL AND audit_items.expected_location_id IS NOT NULL
                              AND EXCLUDED.found_location_id <> audit_items.expected_location_id
                         THEN 'wrong_location' ELSE 'verified' END,
           found_location_id = EXCLUDED.found_location_id, method = 'scan',
           checked_by = EXCLUDED.checked_by, checked_at = now()",
    )
    .bind(id)
    .bind(org)
    .bind(asset_id)
    .bind(found_location)
    .bind(user.id)
    .run(&mut tx)
    .await?;
    let item = find_item(&mut tx, id, asset_id).await?;
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
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    require_open(&find_run(&mut tx, user.org_id, id, true).await?)?;
    let pending = result == "pending";
    let updated = Q::new(
        "UPDATE audit_items SET result = $3, method = $4, note = COALESCE($5, note),
                checked_by = $6, checked_at = CASE WHEN $6::uuid IS NULL THEN NULL ELSE now() END, found_location_id = NULL
          WHERE run_id = $1 AND asset_id = $2",
    )
    .bind(id)
    .bind(asset_id)
    .bind(&result)
    .bind((!pending).then_some("manual"))
    .bind(note)
    .bind((!pending).then_some(user.id))
    .run(&mut tx)
    .await?;
    if updated == 0 {
        return Err(AppError::not_found("Audit item"));
    }
    let item = find_item(&mut tx, id, asset_id).await?;
    tx.commit().await?;
    Ok(Json(item))
}

pub async fn close(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("audits:perform")?;
    let id = path_uuid(&id)?;
    let org = user.org_id;
    let mut tx = state.pool.begin().await?;
    let current = find_run(&mut tx, org, id, true).await?;
    require_open(&current)?;
    // Anything not found by the time the audit closes is recorded as missing.
    Q::new("UPDATE audit_items SET result = 'missing', checked_by = $2, checked_at = now() WHERE run_id = $1 AND result = 'pending'")
        .bind(id)
        .bind(user.id)
        .run(&mut tx)
        .await?;
    let items =
        Q::new("SELECT asset_id, expected, result FROM audit_items WHERE run_id = $1").bind(id).all(&mut tx).await?;
    for item in items {
        let audit = json!({
            "id": id, "name": current["name"], "period": current["period"],
            "result": item["result"], "expected": item["expected"],
        });
        record_event(
            &mut tx,
            org,
            uuid_of(&item["asset_id"]).unwrap(),
            user.id,
            "audited",
            json!({ "audit": { "from": null, "to": audit } }),
        )
        .await?;
    }
    let counts = find_run(&mut tx, org, id, false).await?["counts"].clone();
    Q::new("UPDATE audit_runs SET status = 'closed', closed_by = $2, closed_at = now(), summary = $3 WHERE id = $1")
        .bind(id)
        .bind(user.id)
        .bind(counts)
        .run(&mut tx)
        .await?;
    let run = find_run(&mut tx, org, id, false).await?;
    tx.commit().await?;
    Ok(Json(run))
}
