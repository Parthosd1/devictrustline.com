//! Loaners: items lent from a loaner inventory to a person and back. A loan moves
//! Reserved → Deployed → Returned → Inspected, after which the item is available again (or goes
//! for repair). A reservation can be cancelled; a deployed loan can be extended or reported lost.
//! Each step is recorded in `loan_events` and on the item's history. The item's assignment and
//! `loan_due` follow the loan, so the existing due-back reminders keep working.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

use crate::auth::User;
use crate::db::{uuid_of, Db, Q};
use crate::error::{AppError, Result};
use crate::routes::assets::{find_person, record_event, set_assignment};
use crate::validate::{path_uuid, Input, Str, DATE};
use crate::web::Body;
use crate::AppState;

const SELECT: &str = r#"
  SELECT l.id, l.number, l.status, l.asset_id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName",
         a.inventory_id AS "inventoryId", inv.name AS "inventoryName",
         l.person_id AS "personId", p.name AS "personName", p.email::text AS "personEmail",
         l.starts_on AS "startsOn", l.due_on AS "dueOn", l.purpose,
         l.status = 'deployed' AND l.due_on < (now() AT TIME ZONE o.timezone)::date AS overdue,
         CASE WHEN l.status = 'deployed' THEN (now() AT TIME ZONE o.timezone)::date - l.due_on END AS "daysOverdue",
         l.reserved_at AS "reservedAt", rb.name AS "reservedByName", l.deployed_at AS "deployedAt", db.name AS "deployedByName",
         l.acknowledged_name AS "acknowledgedName", l.acknowledged_at AS "acknowledgedAt",
         l.checkout_condition AS "checkoutCondition", l.returned_at AS "returnedAt", tb.name AS "returnedByName",
         l.return_condition AS "returnCondition", l.return_note AS "returnNote", l.damage_reported AS "damageReported",
         l.inspected_at AS "inspectedAt", ib.name AS "inspectedByName", l.inspection_result AS "inspectionResult",
         l.inspection_note AS "inspectionNote", l.closed_at AS "closedAt", l.extensions
    FROM loans l
    JOIN organizations o ON o.id = l.org_id
    JOIN assets a ON a.id = l.asset_id
    JOIN people p ON p.id = l.person_id
    LEFT JOIN inventories inv ON inv.id = a.inventory_id
    LEFT JOIN users rb ON rb.id = l.reserved_by
    LEFT JOIN users db ON db.id = l.deployed_by
    LEFT JOIN users tb ON tb.id = l.returned_by
    LEFT JOIN users ib ON ib.id = l.inspected_by"#;

const CONDITIONS: &[&str] = &["New", "Good", "Fair", "Poor", "Broken"];

async fn find(db: &mut Db, user: &User, id: Uuid) -> Result<Value> {
    let loan = Q::new(format!("{SELECT} WHERE l.org_id = $1 AND l.id = $2"))
        .bind(user.org_id)
        .bind(id)
        .opt(db)
        .await?
        .filter(|l| user.sees_inventory(uuid_of(&l["inventoryId"])))
        .ok_or_else(|| AppError::not_found("Loan"))?;
    Ok(loan)
}

/// Locks a loan for a change and checks it is in one of the expected states.
async fn lock(db: &mut Db, user: &User, id: Uuid, expected: &[&str]) -> Result<Value> {
    let loan = Q::new(
        r#"SELECT l.id, l.number, l.status, l.asset_id, l.person_id, l.due_on::text AS due_on, l.extensions,
                  l.damage_reported, a.inventory_id AS "inventoryId"
             FROM loans l JOIN assets a ON a.id = l.asset_id WHERE l.org_id = $1 AND l.id = $2 FOR UPDATE OF l"#,
    )
    .bind(user.org_id)
    .bind(id)
    .opt(&mut *db)
    .await?
    .filter(|l| user.sees_inventory(uuid_of(&l["inventoryId"])))
    .ok_or_else(|| AppError::not_found("Loan"))?;
    let status = loan["status"].as_str().unwrap_or("");
    if !expected.contains(&status) {
        return Err(AppError::new(409, format!("This loan is {status}")));
    }
    Ok(loan)
}

async fn event(db: &mut Db, user: &User, loan: Uuid, kind: &str, details: Value) -> Result<()> {
    Q::new("INSERT INTO loan_events (org_id, loan_id, kind, actor_id, details) VALUES ($1, $2, $3, $4, $5)")
        .bind(user.org_id)
        .bind(loan)
        .bind(kind)
        .bind(user.id)
        .bind(details)
        .run(db)
        .await?;
    Ok(())
}

/// The open loan on an item, if any (number and status).
pub async fn open_loan(db: &mut Db, org: Uuid, asset: Uuid) -> Result<Option<Value>> {
    Q::new("SELECT number, status FROM loans WHERE org_id = $1 AND asset_id = $2 AND status IN ('reserved', 'deployed', 'returned')")
        .bind(org)
        .bind(asset)
        .opt(db)
        .await
}

/// Refuses a plain assign or return on an item whose loan is out or awaiting inspection.
pub async fn refuse_if_on_loan(db: &mut Db, org: Uuid, asset: Uuid) -> Result<()> {
    if let Some(loan) = open_loan(db, org, asset).await? {
        if loan["status"] != "reserved" {
            return Err(AppError::new(
                409,
                format!(
                    "This item is on loan {}. Use the loan to return or hand it over.",
                    loan["number"].as_str().unwrap_or("")
                ),
            ));
        }
    }
    Ok(())
}

fn summary_sql(scoped: &str) -> String {
    format!(
        r#"SELECT count(*) FILTER (WHERE l.status = 'reserved')::int AS reserved,
                  count(*) FILTER (WHERE l.status = 'deployed')::int AS deployed,
                  count(*) FILTER (WHERE l.status = 'deployed' AND l.due_on < (now() AT TIME ZONE o.timezone)::date)::int AS overdue,
                  count(*) FILTER (WHERE l.status = 'returned')::int AS "awaitingInspection",
                  count(*) FILTER (WHERE l.status = 'lost')::int AS lost,
                  count(*) FILTER (WHERE l.status = 'inspected' AND l.closed_at > now() - interval '30 days')::int AS "completedLast30Days",
                  round(avg(EXTRACT(EPOCH FROM (l.returned_at - l.deployed_at)) / 86400)
                        FILTER (WHERE l.returned_at IS NOT NULL)::numeric, 1)::float8 AS "averageLoanDays",
                  count(*) FILTER (WHERE l.damage_reported)::int AS "damagedReturns"
             FROM loans l JOIN organizations o ON o.id = l.org_id JOIN assets a ON a.id = l.asset_id
            WHERE l.org_id = $1 AND {scoped}"#
    )
}

async fn search(state: &AppState, user: &User, q: &HashMap<String, String>) -> Result<(Vec<Value>, Value)> {
    user.require("loans:read")?;
    let mut input = Input::query(q);
    let status = input
        .one_of("status", &["reserved", "deployed", "returned", "inspected", "cancelled", "lost", "open"], true, false)
        .into_option();
    let overdue = input.one_of_or("overdue", &["true", "false"], "false") == "true";
    let person = input.uuid("personId", true, false).into_option();
    let asset = input.uuid("assetId", true, false).into_option();
    let limit = input.int("limit", 1, 1000, 200);
    input.done()?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!(
        "{SELECT} WHERE l.org_id = $1 AND {}
            AND ($3::text IS NULL OR l.status = $3 OR ($3 = 'open' AND l.status IN ('reserved', 'deployed', 'returned')))
            AND (NOT $4 OR (l.status = 'deployed' AND l.due_on < (now() AT TIME ZONE o.timezone)::date))
            AND ($5::uuid IS NULL OR l.person_id = $5) AND ($6::uuid IS NULL OR l.asset_id = $6)
          ORDER BY CASE l.status WHEN 'deployed' THEN 0 WHEN 'returned' THEN 1 WHEN 'reserved' THEN 2 ELSE 3 END, l.due_on, l.number
          LIMIT {limit}",
        user.scope_sql("a", 2)
    ))
    .bind(user.org_id)
    .bind(user.scope_param())
    .bind(status)
    .bind(overdue)
    .bind(person)
    .bind(asset)
    .all(&mut db)
    .await?;
    let summary =
        Q::new(summary_sql(&user.scope_sql("a", 2))).bind(user.org_id).bind(user.scope_param()).one(&mut db).await?;
    Ok((items, summary))
}

pub async fn list(
    State(state): State<AppState>,
    user: User,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let (items, summary) = search(&state, &user, &q).await?;
    Ok(Json(json!({ "items": items, "summary": summary })))
}

pub async fn export(
    State(state): State<AppState>,
    user: User,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response> {
    let mut q = q;
    q.entry("limit".into()).or_insert_with(|| "1000".into());
    let (items, _) = search(&state, &user, &q).await?;
    let columns = [
        ("number", "Loan"),
        ("status", "Status"),
        ("assetTag", "Asset tag"),
        ("assetName", "Item"),
        ("personName", "Borrower"),
        ("startsOn", "Starts"),
        ("dueOn", "Due back"),
        ("overdue", "Overdue"),
        ("deployedAt", "Handed over"),
        ("acknowledgedName", "Acknowledged by"),
        ("returnedAt", "Returned"),
        ("returnCondition", "Return condition"),
        ("damageReported", "Damaged"),
        ("inspectionResult", "Inspection"),
        ("extensions", "Extensions"),
        ("purpose", "Purpose"),
    ];
    Ok(crate::routes::assets::csv_response(&columns, &items, "devicetrustline-loans.csv"))
}

pub async fn get(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("loans:read")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let mut loan = find(&mut db, &user, id).await?;
    loan["events"] = json!(
        Q::new(
            r#"SELECT e.id, e.kind, e.details, e.created_at AS "createdAt", u.name AS "actorName"
                 FROM loan_events e LEFT JOIN users u ON u.id = e.actor_id WHERE e.loan_id = $1 ORDER BY e.id"#,
        )
        .bind(id)
        .all(&mut db)
        .await?
    );
    Ok(Json(loan))
}

/// The item for a new loan: in a loaner inventory (or inside one), available, not assigned.
async fn loanable(db: &mut Db, user: &User, asset: Uuid) -> Result<Value> {
    let a = Q::new(
        r#"WITH RECURSIVE up AS (
              SELECT i.id, i.parent_id, i.kind, 0 AS depth FROM inventories i
               WHERE i.id = (SELECT inventory_id FROM assets WHERE id = $2 AND org_id = $1)
              UNION ALL SELECT p.id, p.parent_id, p.kind, up.depth + 1 FROM inventories p JOIN up ON p.id = up.parent_id
               WHERE up.depth < 20)
           SELECT a.id, a.asset_tag, a.name, a.status, a.assigned_person_id, a.archived_at, a.condition,
                  a.inventory_id AS "inventoryId", EXISTS (SELECT 1 FROM up WHERE kind = 'loaner') AS loaner
             FROM assets a WHERE a.org_id = $1 AND a.id = $2 FOR UPDATE OF a"#,
    )
    .bind(user.org_id)
    .bind(asset)
    .opt(&mut *db)
    .await?
    .filter(|a| user.sees_inventory(uuid_of(&a["inventoryId"])))
    .ok_or_else(|| AppError::new(400, "Item not found"))?;
    if !a["archived_at"].is_null() || a["status"] == "Retired" {
        return Err(AppError::new(409, "Archived or retired items cannot be lent"));
    }
    if a["loaner"] != true {
        return Err(AppError::new(409, "Only items in a loaner inventory can be lent. Transfer it there first."));
    }
    if a["status"] != "Available" || !a["assigned_person_id"].is_null() {
        return Err(AppError::new(409, "This item is not available"));
    }
    Ok(a)
}

pub async fn create(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("loans:manage")?;
    let mut input = Input::body(&body);
    let asset = input.uuid("assetId", true, false).into_option();
    let code = input.str("code", Str::req(1, 300).optional()).into_option();
    let person = input.uuid("personId", false, false).into_option().unwrap_or_default();
    let starts = input.str("startsOn", DATE).into_option();
    let due = input.req("dueOn", DATE);
    let purpose = input.str("purpose", Str::text(500)).into_option();
    let deploy_now = input.boolean_or("deployNow", false);
    let acknowledged = input.str("acknowledgedName", Str::text(120)).into_option();
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let asset = match (asset, code) {
        (Some(a), None) => a,
        (None, Some(c)) => {
            let code = crate::routes::assets::parse_scanned_code(&c);
            Q::new("SELECT id FROM assets WHERE org_id = $1 AND (asset_tag = $2 OR serial = $2) ORDER BY asset_tag = $2 DESC LIMIT 1")
                .bind(user.org_id)
                .bind(code)
                .scalars(&mut tx)
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| AppError::new(400, "Item not found"))?
        }
        _ => return Err(AppError::new(400, "Give either assetId or code")),
    };
    let a = loanable(&mut tx, &user, asset).await?;
    let borrower = find_person(&mut tx, user.org_id, person).await?;
    let today: String = Q::new("SELECT (now() AT TIME ZONE timezone)::date::text FROM organizations WHERE id = $1")
        .bind(user.org_id)
        .scalar(&mut tx)
        .await?;
    let starts = starts.unwrap_or_else(|| today.clone());
    if due < starts {
        return Err(AppError::new(400, "The due date must be on or after the start date"));
    }
    if due < today {
        return Err(AppError::new(400, "The due date has already passed"));
    }
    let n: i32 = Q::new(
        "UPDATE organizations SET next_loan_number = next_loan_number + 1 WHERE id = $1 RETURNING next_loan_number - 1",
    )
    .bind(user.org_id)
    .scalar(&mut tx)
    .await?;
    let number = format!("LN-{n}");
    let id: Uuid = Q::new(
        "INSERT INTO loans (org_id, number, asset_id, person_id, status, starts_on, due_on, purpose, reserved_by)
         VALUES ($1, $2, $3, $4, 'reserved', $5::date, $6::date, $7, $8) RETURNING id",
    )
    .bind(user.org_id)
    .bind(&number)
    .bind(asset)
    .bind(borrower.id)
    .bind(&starts)
    .bind(&due)
    .bind(purpose)
    .bind(user.id)
    .scalar(&mut tx)
    .await
    .map_err(|e| {
        if e.constraint() == Some("loans_one_open") {
            AppError::new(409, "This item already has an open loan")
        } else {
            e
        }
    })?;
    event(&mut tx, &user, id, "reserved", json!({ "person": borrower.name, "startsOn": starts, "dueOn": due })).await?;
    record_event(
        &mut tx,
        user.org_id,
        asset,
        user.id,
        "loan_reserved",
        json!({ "loan": { "from": null, "to": number } }),
    )
    .await?;
    if deploy_now {
        deploy_locked(&mut tx, &user, id, acknowledged, a["condition"].as_str().map(String::from), None).await?;
    }
    let loan = find(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(loan)))
}

async fn deploy_locked(
    db: &mut Db,
    user: &User,
    id: Uuid,
    acknowledged: Option<String>,
    condition: Option<String>,
    note: Option<String>,
) -> Result<()> {
    let loan = lock(db, user, id, &["reserved"]).await?;
    let asset = uuid_of(&loan["asset_id"]).unwrap();
    let person = find_person(db, user.org_id, uuid_of(&loan["person_id"]).unwrap()).await?;
    let number = loan["number"].as_str().unwrap_or("").to_string();
    let current: Option<String> =
        Q::new("SELECT condition FROM assets WHERE id = $1").bind(asset).scalars(&mut *db).await?.pop().flatten();
    let condition = condition.or(current);
    set_assignment(db, user, asset, Some(&person), "", true, Some(format!("Loan {number}"))).await?;
    Q::new("UPDATE assets SET loan_due = $3::date WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(asset)
        .bind(loan["due_on"].as_str())
        .run(&mut *db)
        .await?;
    Q::new(
        "UPDATE loans SET status = 'deployed', deployed_by = $3, deployed_at = now(), acknowledged_name = $4,
                acknowledged_at = CASE WHEN $4::text IS NULL THEN NULL ELSE now() END, checkout_condition = $5
          WHERE org_id = $1 AND id = $2",
    )
    .bind(user.org_id)
    .bind(id)
    .bind(user.id)
    .bind(&acknowledged)
    .bind(&condition)
    .run(&mut *db)
    .await?;
    event(db, user, id, "deployed", json!({ "acknowledgedName": acknowledged, "condition": condition, "note": note }))
        .await
}

pub async fn deploy(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("loans:manage")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let acknowledged = input.str("acknowledgedName", Str::text(120)).into_option();
    let condition = input.one_of("condition", CONDITIONS, true, true).into_option();
    let note = input.str("note", Str::text(500)).into_option();
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    deploy_locked(&mut tx, &user, id, acknowledged, condition, note).await?;
    let loan = find(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok(Json(loan))
}

pub async fn extend(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("loans:manage")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let due = input.req("dueOn", DATE);
    let reason = input.req("reason", Str::req(1, 500));
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let loan = lock(&mut tx, &user, id, &["reserved", "deployed"]).await?;
    let old = loan["due_on"].as_str().unwrap_or("").to_string();
    if due <= old {
        return Err(AppError::new(400, "Choose a later due date"));
    }
    // Technicians extend a loan once; further extensions need a manager.
    if loan["extensions"].as_i64().unwrap_or(0) >= 1 && !crate::auth::can(&user.role, "loans:resolve") {
        return Err(AppError::new(403, "This loan was already extended once. Ask a manager to extend it again."));
    }
    Q::new("UPDATE loans SET due_on = $3::date, extensions = extensions + 1 WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(id)
        .bind(&due)
        .run(&mut tx)
        .await?;
    if loan["status"] == "deployed" {
        Q::new("UPDATE assets SET loan_due = $3::date WHERE org_id = $1 AND id = $2")
            .bind(user.org_id)
            .bind(uuid_of(&loan["asset_id"]))
            .bind(&due)
            .run(&mut tx)
            .await?;
    }
    event(&mut tx, &user, id, "extended", json!({ "from": old, "to": due, "reason": reason })).await?;
    let loan = find(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok(Json(loan))
}

pub async fn return_loan(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("loans:manage")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let condition = input.one_of("condition", CONDITIONS, true, true).into_option();
    let damaged = input.boolean_or("damaged", false);
    let note = input.str("note", Str::text(1000)).into_option();
    input.strict()?;
    if damaged && note.is_none() {
        return Err(AppError::new(400, "Describe the damage in a note"));
    }
    let mut tx = state.pool.begin().await?;
    let loan = lock(&mut tx, &user, id, &["deployed"]).await?;
    let asset = uuid_of(&loan["asset_id"]).unwrap();
    let number = loan["number"].as_str().unwrap_or("").to_string();
    // Until it is inspected the item stays out of circulation; damaged items go to maintenance.
    let status = if damaged { "Maintenance" } else { "Available" };
    set_assignment(&mut tx, &user, asset, None, status, true, Some(format!("Loan {number} returned"))).await?;
    Q::new("UPDATE assets SET loan_due = NULL, condition = COALESCE($3, condition) WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(asset)
        .bind(&condition)
        .run(&mut tx)
        .await?;
    Q::new(
        "UPDATE loans SET status = 'returned', returned_by = $3, returned_at = now(), return_condition = $4,
                return_note = $5, damage_reported = $6
          WHERE org_id = $1 AND id = $2",
    )
    .bind(user.org_id)
    .bind(id)
    .bind(user.id)
    .bind(&condition)
    .bind(&note)
    .bind(damaged)
    .run(&mut tx)
    .await?;
    event(&mut tx, &user, id, "returned", json!({ "condition": condition, "damaged": damaged, "note": note })).await?;
    let loan = find(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok(Json(loan))
}

pub async fn inspect(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("loans:manage")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let result = input.one_of("result", &["ready", "repair"], false, false).into_option().unwrap_or_default();
    let note = input.str("note", Str::text(1000)).into_option();
    input.strict()?;
    if result == "repair" && note.is_none() {
        return Err(AppError::new(400, "Say what needs repair in a note"));
    }
    let mut tx = state.pool.begin().await?;
    let loan = lock(&mut tx, &user, id, &["returned"]).await?;
    let asset = uuid_of(&loan["asset_id"]).unwrap();
    let status = if result == "ready" { "Available" } else { "Maintenance" };
    let before: String =
        Q::new("SELECT status FROM assets WHERE id = $1 FOR UPDATE").bind(asset).scalar(&mut tx).await?;
    if before != status {
        Q::new("UPDATE assets SET status = $3, updated_at = now() WHERE org_id = $1 AND id = $2")
            .bind(user.org_id)
            .bind(asset)
            .bind(status)
            .run(&mut tx)
            .await?;
    }
    record_event(
        &mut tx,
        user.org_id,
        asset,
        user.id,
        "loan_inspected",
        json!({ "status": { "from": before, "to": status }, "loan": { "from": null, "to": loan["number"] }, "note": { "from": null, "to": note } }),
    )
    .await?;
    Q::new(
        "UPDATE loans SET status = 'inspected', inspected_by = $3, inspected_at = now(), inspection_result = $4,
                inspection_note = $5, closed_at = now()
          WHERE org_id = $1 AND id = $2",
    )
    .bind(user.org_id)
    .bind(id)
    .bind(user.id)
    .bind(&result)
    .bind(&note)
    .run(&mut tx)
    .await?;
    event(&mut tx, &user, id, "inspected", json!({ "result": result, "note": note })).await?;
    let loan = find(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok(Json(loan))
}

pub async fn cancel(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("loans:manage")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let reason = input.str("reason", Str::text(500)).into_option();
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let loan = lock(&mut tx, &user, id, &["reserved"]).await?;
    Q::new("UPDATE loans SET status = 'cancelled', closed_at = now() WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(id)
        .run(&mut tx)
        .await?;
    event(&mut tx, &user, id, "cancelled", json!({ "reason": reason })).await?;
    record_event(
        &mut tx,
        user.org_id,
        uuid_of(&loan["asset_id"]).unwrap(),
        user.id,
        "loan_cancelled",
        json!({ "loan": { "from": loan["number"], "to": null } }),
    )
    .await?;
    let loan = find(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok(Json(loan))
}

/// A deployed item that will not come back. The item stays recorded with its borrower so the
/// loss can be followed up; only a manager or admin reports it.
pub async fn lost(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("loans:resolve")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let note = input.req("note", Str::req(1, 1000));
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let loan = lock(&mut tx, &user, id, &["deployed"]).await?;
    let asset = uuid_of(&loan["asset_id"]).unwrap();
    Q::new("UPDATE loans SET status = 'lost', closed_at = now() WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(id)
        .run(&mut tx)
        .await?;
    Q::new("UPDATE assets SET loan_due = NULL WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(asset)
        .run(&mut tx)
        .await?;
    event(&mut tx, &user, id, "lost", json!({ "note": note })).await?;
    record_event(
        &mut tx,
        user.org_id,
        asset,
        user.id,
        "loan_lost",
        json!({ "loan": { "from": loan["number"], "to": null }, "note": { "from": null, "to": note } }),
    )
    .await?;
    let loan = find(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok(Json(loan))
}

/// Background job: once per due date, records each overdue loan and tells the managers and admins
/// who see every inventory, and the custodian of the item's inventory.
pub async fn notify_overdue(pool: &sqlx::PgPool, config: &crate::config::Config) -> Result<usize> {
    let roles: Vec<String> =
        crate::auth::ROLES.iter().filter(|r| crate::auth::can(r, "loans:resolve")).map(|r| r.to_string()).collect();
    let mut tx = pool.begin().await?;
    let due = Q::new(
        r#"SELECT l.id, l.org_id, l.number, l.due_on::text AS due_on, l.asset_id, a.name AS asset, a.asset_tag,
                  p.name AS person, inv.custodian_id
             FROM loans l JOIN organizations o ON o.id = l.org_id JOIN assets a ON a.id = l.asset_id
             JOIN people p ON p.id = l.person_id LEFT JOIN inventories inv ON inv.id = a.inventory_id
            WHERE l.status = 'deployed' AND l.due_on < (now() AT TIME ZONE o.timezone)::date
              AND NOT EXISTS (SELECT 1 FROM loan_events e WHERE e.loan_id = l.id AND e.kind = 'overdue'
                                AND e.details->>'dueOn' = l.due_on::text)
            ORDER BY l.due_on LIMIT 200
            FOR UPDATE OF l SKIP LOCKED"#,
    )
    .all(&mut tx)
    .await?;
    for l in &due {
        let org = uuid_of(&l["org_id"]).unwrap();
        let loan = uuid_of(&l["id"]).unwrap();
        Q::new("INSERT INTO loan_events (org_id, loan_id, kind, details) VALUES ($1, $2, 'overdue', $3)")
            .bind(org)
            .bind(loan)
            .bind(json!({ "dueOn": l["due_on"] }))
            .run(&mut tx)
            .await?;
        let recipients: Vec<Uuid> = Q::new(
            "SELECT m.user_id FROM memberships m JOIN users u ON u.id = m.user_id
              WHERE m.org_id = $1 AND m.is_active AND u.is_active
                AND ((m.role = ANY($2) AND m.access_scope = 'all') OR m.user_id = $3)",
        )
        .bind(org)
        .bind(roles.clone())
        .bind(uuid_of(&l["custodian_id"]))
        .scalars(&mut tx)
        .await?;
        let title =
            format!("Overdue loan {}: {}", l["number"].as_str().unwrap_or(""), l["asset"].as_str().unwrap_or(""));
        let body = format!(
            "{} ({}) was due back from {} on {}.",
            l["asset"].as_str().unwrap_or(""),
            l["asset_tag"].as_str().unwrap_or(""),
            l["person"].as_str().unwrap_or(""),
            l["due_on"].as_str().unwrap_or("")
        );
        for user in recipients {
            let notice = crate::notify::Notice {
                asset_id: uuid_of(&l["asset_id"]),
                link: Some("/?page=Loaners".into()),
                dedupe_key: Some(format!("loan-overdue:{loan}:{}:{user}", l["due_on"].as_str().unwrap_or(""))),
                ..crate::notify::Notice::new(org, user, "loan_overdue", &title, &body)
            };
            crate::notify::send(&mut tx, config, notice).await?;
        }
    }
    tx.commit().await?;
    Ok(due.len())
}
