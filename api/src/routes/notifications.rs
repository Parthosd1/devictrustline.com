//! The notification center, upcoming deadlines, and insights.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use crate::auth::User;
use crate::db::Q;
use crate::error::{AppError, Result};
use crate::reminders::DEADLINES;
use crate::validate::Input;
use crate::AppState;

pub async fn list(
    State(state): State<AppState>,
    user: User,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let unread = q.get("unread").is_some_and(|v| v == "true" || v == "1");
    let mut input = Input::query(&q);
    input.str("unread", crate::validate::Str::req(0, 5).optional());
    let limit = input.int("limit", 1, 200, 50);
    input.done()?;
    let mut db = state.pool.acquire().await?;
    // emailStatus is what the email system recorded: queued, sent (accepted by the provider),
    // failed, or unavailable (no email provider set up). Null when no email was due.
    let items = Q::new(format!(
        r#"SELECT n.id, n.kind, n.title, n.body, n.asset_id AS "assetId", n.created_at AS "createdAt",
                  n.read_at AS "readAt", CASE WHEN e.status = 'sending' THEN 'queued' ELSE e.status END AS "emailStatus"
             FROM notifications n LEFT JOIN email_outbox e ON e.id = n.email_id
            WHERE n.user_id = $1 AND n.org_id = $2 AND ($3 = false OR n.read_at IS NULL)
            ORDER BY n.created_at DESC, n.id DESC LIMIT {limit}"#
    ))
    .bind(user.id)
    .bind(user.org_id)
    .bind(unread)
    .all(&mut db)
    .await?;
    let unread_count: i64 =
        Q::new("SELECT count(*) FROM notifications WHERE user_id = $1 AND org_id = $2 AND read_at IS NULL")
            .bind(user.id)
            .bind(user.org_id)
            .scalar(&mut db)
            .await?;
    Ok(Json(json!({ "items": items, "unread": unread_count })))
}

pub async fn read(State(state): State<AppState>, user: User, Path(id): Path<i64>) -> Result<StatusCode> {
    let mut db = state.pool.acquire().await?;
    let n = Q::new(
        "UPDATE notifications SET read_at = COALESCE(read_at, now()) WHERE id = $1 AND user_id = $2 AND org_id = $3",
    )
    .bind(id)
    .bind(user.id)
    .bind(user.org_id)
    .run(&mut db)
    .await?;
    if n == 0 {
        return Err(AppError::not_found("Notification"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn read_all(State(state): State<AppState>, user: User) -> Result<StatusCode> {
    let mut db = state.pool.acquire().await?;
    Q::new("UPDATE notifications SET read_at = now() WHERE user_id = $1 AND org_id = $2 AND read_at IS NULL")
        .bind(user.id)
        .bind(user.org_id)
        .run(&mut db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Upcoming (and recently missed) deadlines in this workspace.
pub async fn deadlines(
    State(state): State<AppState>,
    user: User,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let mut input = Input::query(&q);
    let days = input.int("days", 1, 3650, 90);
    input.done()?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!(
        r#"WITH d AS ({DEADLINES})
           SELECT d.kind, d.source_id AS "sourceId", d.asset_id AS "assetId", d.title, d.due_on AS "dueOn",
                  d.due_on - (now() AT TIME ZONE o.timezone)::date AS "daysLeft"
             FROM d JOIN organizations o ON o.id = d.org_id
            WHERE d.org_id = $1 AND d.due_on <= (now() AT TIME ZONE o.timezone)::date + {days}
              AND d.due_on >= (now() AT TIME ZONE o.timezone)::date - 30
            ORDER BY d.due_on, d.title LIMIT 200"#
    ))
    .bind(user.org_id)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items, "days": days })))
}

/// Suggestions worth a look, each with the reason it was made. Nothing is changed or listed for
/// sale automatically; these only point at items.
pub async fn insights(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let mut db = state.pool.acquire().await?;
    let org = user.org_id;
    let today = "(SELECT (now() AT TIME ZONE timezone)::date FROM organizations WHERE id = $1)";
    let item = r#"jsonb_build_object('id', a.id, 'assetTag', a.asset_tag, 'name', a.name, 'type', a.type)"#;
    let queries: Vec<(&str, &str, String, String)> = vec![
        (
            "redeployable",
            "Unassigned items you could put back to use",
            format!(
                "SELECT {item} FROM assets a WHERE a.org_id = $1 AND a.status = 'Available' AND a.assigned_person_id IS NULL
                   AND a.updated_at < now() - interval '30 days' ORDER BY a.updated_at LIMIT 50"
            ),
            "These items are marked Available, are not assigned to anyone, and have not changed in over 30 days.".into(),
        ),
        (
            "warranty_ending",
            "Warranties ending in the next 60 days",
            format!(
                "SELECT ({item} || jsonb_build_object('endsOn', w.ends_on)) FROM warranties w JOIN assets a ON a.id = w.asset_id
                  WHERE w.org_id = $1 AND a.status <> 'Retired' AND w.ends_on BETWEEN {today} AND {today} + 60
                  ORDER BY w.ends_on LIMIT 50"
            ),
            "A warranty on each of these ends within 60 days. Check for problems while repairs are still covered.".into(),
        ),
        (
            "frequent_repairs",
            "Items repaired often",
            format!(
                "SELECT ({item} || jsonb_build_object('workOrders', count(*))) FROM work_orders wo JOIN assets a ON a.id = wo.asset_id
                  WHERE wo.org_id = $1 AND wo.opened_at > now() - interval '365 days'
                  GROUP BY a.id HAVING count(*) >= 3 ORDER BY count(*) DESC LIMIT 50"
            ),
            "Each of these had 3 or more work orders in the past year. Replacing one can cost less than repairing it again.".into(),
        ),
        (
            "overdue_returns",
            "Loans that are overdue",
            format!(
                "SELECT ({item} || jsonb_build_object('loanDue', a.loan_due, 'person', p.name))
                   FROM assets a JOIN people p ON p.id = a.assigned_person_id
                  WHERE a.org_id = $1 AND a.loan_due < {today} ORDER BY a.loan_due LIMIT 50"
            ),
            "These items were due back and are still assigned.".into(),
        ),
        (
            "return_window_closing",
            "Return windows closing soon",
            format!(
                "SELECT ({item} || jsonb_build_object('returnDeadline', a.return_deadline)) FROM assets a
                  WHERE a.org_id = $1 AND a.return_deadline BETWEEN {today} AND {today} + 7 ORDER BY a.return_deadline LIMIT 50"
            ),
            "The last day to return each of these is within a week, if you meant to.".into(),
        ),
        (
            "surplus",
            "Possible surplus",
            "SELECT jsonb_build_object('type', a.type, 'available', count(*)) FROM assets a
              WHERE a.org_id = $1 AND a.status = 'Available' AND a.assigned_person_id IS NULL
              GROUP BY a.type HAVING count(*) >= 3 ORDER BY count(*) DESC LIMIT 20"
                .into(),
            "You have 3 or more spare items in these categories. You might keep fewer on hand, sell or donate some — nothing is listed for you.".into(),
        ),
    ];
    let mut out = vec![];
    for (kind, title, sql, why) in queries {
        let items: Vec<Value> = Q::new(sql).bind(org).scalars(&mut db).await?;
        if !items.is_empty() {
            out.push(json!({ "kind": kind, "title": title, "explanation": why, "items": items }));
        }
    }
    Ok(Json(json!({ "items": out })))
}
