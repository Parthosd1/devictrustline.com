//! The manager approval center: everything waiting on a decision or a follow-up, in one place.
//! Each section appears only to people allowed to act on it, and only for items inside their
//! access. Decisions are made through each area's own routes (transfers, audits, access requests,
//! loans); this queue only gathers them. Follow-up tasks track work raised from the queue.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use uuid::Uuid;

use crate::auth::{can, User};
use crate::db::{uuid_of, Db, Q};
use crate::error::{AppError, Result};
use crate::validate::{path_uuid, Input, Str, DATE};
use crate::web::Body;
use crate::AppState;

/// Whether an open follow-up task already covers this item ($alias.asset id) for a kind.
fn has_task(asset: &str, kind: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM follow_up_tasks f WHERE f.org_id = $1 AND f.status = 'open' AND f.asset_id = {asset} AND f.kind = '{kind}') AS \"hasTask\""
    )
}

pub async fn queue(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    let mut db = state.pool.acquire().await?;
    let mut out = Map::new();
    let scope = user.scope_param();
    let a_scope = "($2::uuid[] IS NULL OR a.inventory_id = ANY($2))";
    let today = "(now() AT TIME ZONE o.timezone)::date";
    let mut section = |name: &str, rows: Vec<Value>| {
        out.insert(name.to_string(), json!(rows));
    };

    if can(&user.role, "transfers:approve") {
        section(
            "transfers",
            Q::new(
                r#"SELECT t.id, t.batch_id AS "batchId", a.id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName",
                          fi.name AS "fromName", ti.name AS "toName", t.reason, u.name AS "requestedBy",
                          t.requested_at AS "requestedAt", t.requested_by = $3 AS mine
                     FROM inventory_transfers t JOIN assets a ON a.id = t.asset_id
                     JOIN inventories ti ON ti.id = t.to_inventory_id LEFT JOIN inventories fi ON fi.id = t.from_inventory_id
                     JOIN users u ON u.id = t.requested_by
                    WHERE t.org_id = $1 AND t.status = 'pending' AND t.via_asset_id IS NULL
                      AND ($2::uuid[] IS NULL OR t.to_inventory_id = ANY($2) OR t.from_inventory_id = ANY($2))
                    ORDER BY t.requested_at LIMIT 200"#
            )
            .bind(user.org_id)
            .bind(&scope)
            .bind(user.id)
            .all(&mut db)
            .await?,
        );
    }
    if can(&user.role, "audits:approve") {
        section(
            "audits",
            Q::new(
                r#"SELECT r.id, r.name, r.audit_type AS "auditType", r.period, inv.name AS "inventoryName",
                          r.submitted_at AS "submittedAt", su.name AS "submittedBy",
                          (SELECT count(*) FROM audit_items i WHERE i.run_id = r.id AND i.result = 'missing')::int AS missing,
                          (SELECT count(*) FROM audit_items i WHERE i.run_id = r.id AND i.damaged)::int AS damaged,
                          NOT (r.started_by = $3 OR r.submitted_by = $3) AS "canReview"
                     FROM audit_runs r LEFT JOIN inventories inv ON inv.id = r.inventory_id LEFT JOIN users su ON su.id = r.submitted_by
                    WHERE r.org_id = $1 AND r.status = 'submitted' AND ($2::uuid[] IS NULL OR r.inventory_id = ANY($2))
                    ORDER BY r.submitted_at LIMIT 200"#,
            )
            .bind(user.org_id)
            .bind(&scope)
            .bind(user.id)
            .all(&mut db)
            .await?,
        );
    }
    if can(&user.role, "access:approve") && user.scope.is_none() {
        section(
            "accessRequests",
            Q::new(
                r#"SELECT r.id, u.name AS "userName", i.name AS "inventoryName", r.note, r.created_at AS "createdAt"
                     FROM access_requests r JOIN users u ON u.id = r.user_id JOIN inventories i ON i.id = r.inventory_id
                    WHERE r.org_id = $1 AND r.status = 'pending' AND r.user_id <> $2 ORDER BY r.created_at LIMIT 200"#,
            )
            .bind(user.org_id)
            .bind(user.id)
            .all(&mut db)
            .await?,
        );
    }
    if can(&user.role, "loans:read") {
        section(
            "overdueLoans",
            Q::new(format!(
                r#"SELECT l.id, l.number, a.id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName", p.name AS "personName",
                          l.due_on AS "dueOn", {today} - l.due_on AS "daysOverdue", {}
                     FROM loans l JOIN organizations o ON o.id = l.org_id JOIN assets a ON a.id = l.asset_id JOIN people p ON p.id = l.person_id
                    WHERE l.org_id = $1 AND l.status = 'deployed' AND l.due_on < {today} AND {a_scope}
                    ORDER BY l.due_on LIMIT 200"#,
                has_task("a.id", "overdue_loan")
            ))
            .bind(user.org_id)
            .bind(&scope)
            .all(&mut db)
            .await?,
        );
        section(
            "loansToInspect",
            Q::new(format!(
                r#"SELECT l.id, l.number, a.id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName", p.name AS "personName",
                          l.returned_at AS "returnedAt", l.damage_reported AS "damageReported", l.return_note AS "returnNote"
                     FROM loans l JOIN assets a ON a.id = l.asset_id JOIN people p ON p.id = l.person_id
                    WHERE l.org_id = $1 AND l.status = 'returned' AND {a_scope} ORDER BY l.returned_at LIMIT 200"#
            ))
            .bind(user.org_id)
            .bind(&scope)
            .all(&mut db)
            .await?,
        );
    }
    if can(&user.role, "audits:read") {
        // Missing in the latest finished audit that covered the item, and not found since.
        section(
            "missing",
            Q::new(format!(
                r#"SELECT * FROM (
                     SELECT DISTINCT ON (i.asset_id) a.id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName",
                            r.id AS "auditId", r.name AS "auditName", r.closed_at AS "since", i.result, {}
                       FROM audit_items i JOIN audit_runs r ON r.id = i.run_id JOIN assets a ON a.id = i.asset_id
                      WHERE i.org_id = $1 AND r.status IN ('closed', 'approved') AND a.archived_at IS NULL AND {a_scope}
                      ORDER BY i.asset_id, r.closed_at DESC) latest
                   WHERE latest.result = 'missing' ORDER BY "since" DESC LIMIT 200"#,
                has_task("a.id", "missing")
            ))
            .bind(user.org_id)
            .bind(&scope)
            .all(&mut db)
            .await?,
        );
        section(
            "discrepancies",
            Q::new(format!(
                r#"SELECT a.id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName", r.id AS "auditId", r.name AS "auditName",
                          CASE WHEN NOT i.expected THEN 'unexpected' ELSE i.result END AS kind,
                          el.name AS "expectedLocation", fl.name AS "foundLocation", r.closed_at AS "at", {}
                     FROM audit_items i JOIN audit_runs r ON r.id = i.run_id JOIN assets a ON a.id = i.asset_id
                     LEFT JOIN locations el ON el.id = i.expected_location_id LEFT JOIN locations fl ON fl.id = i.found_location_id
                    WHERE i.org_id = $1 AND r.status IN ('closed', 'approved') AND r.closed_at > now() - interval '90 days'
                      AND (i.result = 'wrong_location' OR NOT i.expected) AND {a_scope}
                    ORDER BY r.closed_at DESC LIMIT 200"#,
                has_task("a.id", "discrepancy")
            ))
            .bind(user.org_id)
            .bind(&scope)
            .all(&mut db)
            .await?,
        );
        section(
            "damaged",
            Q::new(format!(
                r#"SELECT * FROM (
                     SELECT a.id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName", 'audit' AS source, r.name AS "sourceName",
                            i.note, r.closed_at AS "at", {h}
                       FROM audit_items i JOIN audit_runs r ON r.id = i.run_id JOIN assets a ON a.id = i.asset_id
                      WHERE i.org_id = $1 AND i.damaged AND r.status IN ('closed', 'approved') AND r.closed_at > now() - interval '90 days'
                        AND a.status <> 'Retired' AND {a_scope}
                     UNION ALL
                     SELECT a.id, a.asset_tag, a.name, 'loan', l.number, l.return_note, l.returned_at, {h}
                       FROM loans l JOIN assets a ON a.id = l.asset_id
                      WHERE l.org_id = $1 AND l.damage_reported AND l.returned_at > now() - interval '90 days'
                        AND a.status <> 'Retired' AND {a_scope}) d
                   ORDER BY "at" DESC LIMIT 200"#,
                h = has_task("a.id", "damaged")
            ))
            .bind(user.org_id)
            .bind(&scope)
            .all(&mut db)
            .await?,
        );
    }
    if can(&user.role, "assets:read") {
        section(
            "warranties",
            Q::new(format!(
                r#"SELECT w.id, a.id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName", w.kind, w.provider,
                          w.ends_on AS "endsOn", w.ends_on - {today} AS "daysLeft"
                     FROM warranties w JOIN assets a ON a.id = w.asset_id JOIN organizations o ON o.id = w.org_id
                    WHERE w.org_id = $1 AND w.ends_on BETWEEN {today} - 7 AND {today} + 30
                      AND a.archived_at IS NULL AND a.status <> 'Retired' AND {a_scope}
                    ORDER BY w.ends_on LIMIT 200"#
            ))
            .bind(user.org_id)
            .bind(&scope)
            .all(&mut db)
            .await?,
        );
    }
    if can(&user.role, "maintenance:read") {
        section(
            "maintenance",
            Q::new(format!(
                r#"SELECT * FROM (
                     SELECT 'work_order' AS source, w.id, w.number, w.title, a.id AS "assetId", a.asset_tag AS "assetTag",
                            a.name AS "assetName", w.due_date AS "dueOn", w.priority
                       FROM work_orders w JOIN assets a ON a.id = w.asset_id JOIN organizations o ON o.id = w.org_id
                      WHERE w.org_id = $1 AND w.status NOT IN ('resolved', 'cancelled') AND w.due_date <= {today} + 7 AND {a_scope}
                     UNION ALL
                     SELECT 'scheduled', a.id, NULL, 'Maintenance due', a.id, a.asset_tag, a.name, a.maintenance_due, NULL
                       FROM assets a JOIN organizations o ON o.id = a.org_id
                      WHERE a.org_id = $1 AND a.maintenance_due <= {today} + 7 AND a.archived_at IS NULL
                        AND a.status <> 'Retired' AND {a_scope}) m
                   ORDER BY "dueOn" LIMIT 200"#
            ))
            .bind(user.org_id)
            .bind(&scope)
            .all(&mut db)
            .await?,
        );
    }
    let counts: Map<String, Value> =
        out.iter().map(|(k, v)| (k.clone(), json!(v.as_array().map(|a| a.len()).unwrap_or(0)))).collect();
    let open_tasks: i64 = Q::new(
        "SELECT count(*) FROM follow_up_tasks f LEFT JOIN assets a ON a.id = f.asset_id
          WHERE f.org_id = $1 AND f.status = 'open' AND ($2::uuid[] IS NULL OR a.inventory_id = ANY($2) OR f.assignee_id = $3)",
    )
    .bind(user.org_id)
    .bind(&scope)
    .bind(user.id)
    .scalar(&mut db)
    .await?;
    Ok(Json(json!({ "sections": out, "counts": counts, "openTasks": open_tasks })))
}

const TASK_SELECT: &str = r#"
  SELECT f.id, f.title, f.notes, f.kind, f.source_type AS "sourceType", f.source_id AS "sourceId",
         f.asset_id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName", a.inventory_id AS "inventoryId",
         f.assignee_id AS "assigneeId", au.name AS "assigneeName", f.due_on AS "dueOn", f.status, f.resolution,
         f.created_at AS "createdAt", cu.name AS "createdBy", f.closed_at AS "closedAt"
    FROM follow_up_tasks f LEFT JOIN assets a ON a.id = f.asset_id
    LEFT JOIN users au ON au.id = f.assignee_id LEFT JOIN users cu ON cu.id = f.created_by"#;

const KINDS: &[&str] =
    &["general", "missing", "damaged", "discrepancy", "overdue_loan", "warranty", "maintenance", "audit", "transfer"];
const SOURCES: &[&str] = &["audit", "transfer", "loan", "work_order", "warranty", "access_request"];

/// A task is visible to people who see its item (or everyone, without one), and to its assignee.
fn visible(user: &User, task: &Value) -> bool {
    uuid_of(&task["assigneeId"]) == Some(user.id)
        || (task["assetId"].is_null() && user.scope.is_none())
        || (!task["assetId"].is_null() && user.sees_inventory(uuid_of(&task["inventoryId"])))
}

async fn find_task(db: &mut Db, user: &User, id: Uuid) -> Result<Value> {
    Q::new(format!("{TASK_SELECT} WHERE f.org_id = $1 AND f.id = $2"))
        .bind(user.org_id)
        .bind(id)
        .opt(db)
        .await?
        .filter(|t| visible(user, t))
        .ok_or_else(|| AppError::not_found("Task"))
}

pub async fn list_tasks(
    State(state): State<AppState>,
    user: User,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    user.require("tasks:read")?;
    let mut input = Input::query(&q);
    let status = input.one_of_or("status", &["open", "done", "cancelled", "all"], "open");
    let mine = input.one_of_or("mine", &["true", "false"], "false") == "true";
    let asset = input.uuid("assetId", true, false).into_option();
    input.done()?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!(
        "{TASK_SELECT} WHERE f.org_id = $1 AND ($2 = 'all' OR f.status = $2) AND (NOT $3 OR f.assignee_id = $4)
            AND ($5::uuid IS NULL OR f.asset_id = $5)
            AND (f.assignee_id = $4 OR ($6::uuid[] IS NULL) OR a.inventory_id = ANY($6))
          ORDER BY f.status = 'open' DESC, f.due_on NULLS LAST, f.created_at DESC LIMIT 500"
    ))
    .bind(user.org_id)
    .bind(status)
    .bind(mine)
    .bind(user.id)
    .bind(asset)
    .bind(user.scope_param())
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

async fn check_assignee(db: &mut Db, org: Uuid, assignee: Option<Uuid>) -> Result<()> {
    if let Some(a) = assignee {
        let ok: i64 = Q::new("SELECT count(*) FROM memberships WHERE org_id = $1 AND user_id = $2 AND is_active")
            .bind(org)
            .bind(a)
            .scalar(&mut *db)
            .await?;
        if ok == 0 {
            return Err(AppError::new(400, "The assignee must be an active member of this workspace"));
        }
    }
    Ok(())
}

pub async fn create_task(
    State(state): State<AppState>,
    user: User,
    Body(body): Body,
) -> Result<(StatusCode, Json<Value>)> {
    user.require("tasks:manage")?;
    let mut input = Input::body(&body);
    let title = input.req("title", Str::req(1, 200));
    let notes = input.str("notes", Str::text(2000)).into_option();
    let kind = input.one_of_or("kind", KINDS, "general");
    let source_type = input.one_of("sourceType", SOURCES, true, true).into_option();
    let source_id = input.uuid("sourceId", true, true).into_option();
    let asset = input.uuid("assetId", true, true).into_option();
    let assignee = input.uuid("assigneeId", true, true).into_option();
    let due = input.str("dueOn", DATE).into_option();
    input.strict()?;
    if source_type.is_some() != source_id.is_some() {
        return Err(AppError::new(400, "Give both sourceType and sourceId, or neither"));
    }
    let mut tx = state.pool.begin().await?;
    if let Some(a) = asset {
        let inv: Option<Option<Uuid>> = Q::new("SELECT inventory_id FROM assets WHERE org_id = $1 AND id = $2")
            .bind(user.org_id)
            .bind(a)
            .scalars(&mut tx)
            .await?
            .pop();
        match inv {
            Some(i) if user.sees_inventory(i) => {}
            _ => return Err(AppError::new(400, "Item not found")),
        }
    } else if user.scope.is_some() {
        return Err(AppError::new(400, "Choose the item this task is about"));
    }
    check_assignee(&mut tx, user.org_id, assignee).await?;
    let id: Uuid = Q::new(
        "INSERT INTO follow_up_tasks (org_id, title, notes, kind, source_type, source_id, asset_id, assignee_id, due_on, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::date, $10) RETURNING id",
    )
    .bind(user.org_id)
    .bind(&title)
    .bind(notes)
    .bind(&kind)
    .bind(source_type)
    .bind(source_id)
    .bind(asset)
    .bind(assignee)
    .bind(due)
    .bind(user.id)
    .scalar(&mut tx)
    .await
    .map_err(|e| {
        if e.constraint() == Some("follow_up_tasks_one_open") {
            AppError::new(409, "An open task already covers this")
        } else {
            e
        }
    })?;
    let task = find_task(&mut tx, &user, id).await?;
    tx.commit().await?;
    if let Some(a) = assignee.filter(|a| *a != user.id) {
        crate::events::send_all(
            &state,
            user.org_id,
            vec![crate::events::Event {
                user: a,
                kind: "task_assigned",
                title: format!("Task assigned: {title}"),
                body: format!("{} assigned you a follow-up task: {title}.", user.name),
                asset,
                link: Some("/?page=Approvals".into()),
                key: format!("task-assigned:{id}:{a}"),
            }],
        )
        .await;
    }
    Ok((StatusCode::CREATED, Json(task)))
}

pub async fn update_task(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("tasks:read")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let title = input.str("title", Str::req(1, 200).optional()).into_option();
    let notes = input.str("notes", Str::text(2000));
    let assignee = input.uuid("assigneeId", true, true);
    let due = input.str("dueOn", DATE);
    let status = input.one_of("status", &["open", "done", "cancelled"], true, false).into_option();
    let resolution = input.str("resolution", Str::text(2000)).into_option();
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let task = find_task(&mut tx, &user, id).await?;
    // Managers change anything; the assignee may only close their own task.
    let manager = crate::auth::can(&user.role, "tasks:manage");
    let editing = title.is_some() || notes.is_present() || assignee.is_present() || due.is_present();
    if !manager && (editing || uuid_of(&task["assigneeId"]) != Some(user.id)) {
        return Err(AppError::new(403, "You do not have permission to do that"));
    }
    if status.as_deref() == Some("done") && resolution.is_none() {
        return Err(AppError::new(400, "Say how the task was resolved"));
    }
    if let Some(a) = assignee.value() {
        check_assignee(&mut tx, user.org_id, Some(*a)).await?;
    }
    Q::new(
        "UPDATE follow_up_tasks SET
           title = COALESCE($3, title),
           notes = CASE WHEN $4 THEN $5 ELSE notes END,
           assignee_id = CASE WHEN $6 THEN $7 ELSE assignee_id END,
           due_on = CASE WHEN $8 THEN $9::date ELSE due_on END,
           status = COALESCE($10, status),
           resolution = CASE WHEN $10 IS NULL THEN resolution WHEN $10 = 'open' THEN NULL ELSE $11 END,
           closed_by = CASE WHEN $10 IS NULL THEN closed_by WHEN $10 = 'open' THEN NULL ELSE $12 END,
           closed_at = CASE WHEN $10 IS NULL THEN closed_at WHEN $10 = 'open' THEN NULL ELSE now() END
         WHERE org_id = $1 AND id = $2",
    )
    .bind(user.org_id)
    .bind(id)
    .bind(title)
    .bind(notes.is_present())
    .bind(notes.into_option())
    .bind(assignee.is_present())
    .bind(assignee.into_option())
    .bind(due.is_present())
    .bind(due.into_option())
    .bind(status)
    .bind(resolution)
    .bind(user.id)
    .run(&mut tx)
    .await
    .map_err(|e| {
        if e.constraint() == Some("follow_up_tasks_one_open") {
            AppError::new(409, "Another open task already covers this")
        } else {
            e
        }
    })?;
    let task = find_task(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok(Json(task))
}
