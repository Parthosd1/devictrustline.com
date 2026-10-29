//! Reminders a person manages for themselves: warranty, return, maintenance, renewal,
//! inspection, loan return and custom dates, once or recurring, with their own "days before"
//! timing. Each person sees and changes only their own reminders in the current workspace.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::User;
use crate::db::{Db, Q};
use crate::error::{AppError, Result};
use crate::user_reminders::{next_due, source_column, DATE_SOURCES, KINDS, REPEATS};
use crate::validate::{path_uuid, Input, Opt, Str, DATE};
use crate::web::Body;
use crate::AppState;

const SELECT: &str = r#"
  SELECT r.id, r.kind, r.title, r.notes, r.asset_id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName",
         r.date_source AS "dateSource", r.due_on AS "dueOn", r.offsets, r.repeat, r.repeat_every AS "repeatEvery",
         r.repeat_until AS "repeatUntil", r.status, r.snoozed_until AS "snoozedUntil", r.completed_at AS "completedAt",
         r.created_at AS "createdAt", r.updated_at AS "updatedAt",
         r.due_on - (now() AT TIME ZONE COALESCE(s.timezone, o.timezone))::date AS "daysLeft",
         (SELECT min(j.fire_at) FROM reminder_jobs j WHERE j.reminder_id = r.id AND j.status = 'pending') AS "nextNotificationAt",
         (SELECT max(j.fired_at) FROM reminder_jobs j WHERE j.reminder_id = r.id AND j.status = 'sent') AS "lastNotifiedAt"
    FROM user_reminders r
    JOIN organizations o ON o.id = r.org_id
    LEFT JOIN notification_settings s ON s.user_id = r.user_id
    LEFT JOIN assets a ON a.id = r.asset_id"#;

async fn find(db: &mut Db, user: &User, id: Uuid) -> Result<Value> {
    Q::new(format!("{SELECT} WHERE r.id = $1 AND r.user_id = $2 AND r.org_id = $3"))
        .bind(id)
        .bind(user.id)
        .bind(user.org_id)
        .opt(db)
        .await?
        .ok_or_else(|| AppError::not_found("Reminder"))
}

/// Days-before offsets: 0 (same day) to 365, at most 8.
fn offsets(input: &mut Input, optional: bool) -> Option<Vec<i32>> {
    let raw = match input.raw_value("offsets") {
        None if optional => return None,
        None => {
            input.fail("offsets", "Required");
            return None;
        }
        Some(v) => v,
    };
    let list: Option<Vec<i32>> = raw
        .as_array()
        .and_then(|a| a.iter().map(|v| v.as_i64().filter(|n| (0..=365).contains(n)).map(|n| n as i32)).collect());
    match list {
        Some(mut l) if (1..=8).contains(&l.len()) => {
            l.sort_unstable_by(|a, b| b.cmp(a));
            l.dedup();
            Some(l)
        }
        _ => {
            input.fail("offsets", "List 1 to 8 whole numbers of days before, from 0 (same day) to 365");
            None
        }
    }
}

fn default_title(kind: &str, asset: Option<&str>) -> String {
    let what = match kind {
        "warranty" => "warranty ends",
        "return" => "last day to return",
        "maintenance" => "maintenance due",
        "renewal" => "subscription or protection plan renews",
        "inspection" => "inspection due",
        "loan_return" => "loaned item due back",
        _ => "reminder",
    };
    match asset {
        Some(name) => format!("{name}: {what}"),
        None => {
            let mut s = what.to_string();
            s[..1].make_ascii_uppercase();
            s
        }
    }
}

/// The date an item gives a reminder that follows it.
async fn source_date(db: &mut Db, org: Uuid, asset: Uuid, source: &str) -> Result<Option<String>> {
    let row =
        Q::new(format!("SELECT {}::text AS date FROM assets WHERE org_id = $1 AND id = $2", source_column(source)))
            .bind(org)
            .bind(asset)
            .opt(db)
            .await?
            .ok_or_else(|| AppError::new(400, "Item not found"))?;
    Ok(row["date"].as_str().map(str::to_string))
}

pub async fn list(
    State(state): State<AppState>,
    user: User,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let mut input = Input::query(&q);
    let status = input.one_of("status", &["active", "paused", "completed", "all"], true, false);
    let asset = input.uuid("assetId", true, false).into_option();
    let kind = input.one_of("kind", KINDS, true, false).into_option();
    input.done()?;
    let status = status.into_option();
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!(
        "{SELECT} WHERE r.user_id = $1 AND r.org_id = $2
            AND ($3::text IS NULL OR $3 = 'all' OR r.status = $3) AND ($3::text IS NOT NULL OR r.status <> 'completed')
            AND ($4::uuid IS NULL OR r.asset_id = $4) AND ($5::text IS NULL OR r.kind = $5)
          ORDER BY r.status = 'completed', r.due_on, r.title LIMIT 500"
    ))
    .bind(user.id)
    .bind(user.org_id)
    .bind(status)
    .bind(asset)
    .bind(kind)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn get(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let mut reminder = find(&mut db, &user, id).await?;
    // What happened to each scheduled notification, and to each channel's delivery.
    reminder["history"] = Q::new(format!(
        r#"SELECT j.id, j.due_on AS "dueOn", j.days_before AS "daysBefore", j.fire_at AS "fireAt", j.status,
                  j.attempts, j.last_error AS "error", j.fired_at AS "firedAt",
                  (SELECT ({}) FROM notifications n WHERE n.id = j.notification_id) AS deliveries
             FROM reminder_jobs j WHERE j.reminder_id = $1 ORDER BY j.fire_at DESC, j.id DESC LIMIT 100"#,
        crate::notify::DELIVERIES_SQL
    ))
    .bind(id)
    .all(&mut db)
    .await
    .map(|rows| json!(rows))?;
    Ok(Json(reminder))
}

pub async fn create(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    let mut input = Input::body(&body);
    let kind = input.one_of("kind", KINDS, false, false).into_option().unwrap_or_default();
    let title = input.str("title", Str::text(160)).into_option();
    let notes = input.str("notes", Str::text(1000)).into_option();
    let asset = input.uuid("assetId", true, true).into_option();
    let source = input.one_of("dateSource", DATE_SOURCES, true, true).into_option();
    let due = input.str("dueOn", DATE).into_option();
    let offs = offsets(&mut input, true);
    let repeat = input.one_of_or("repeat", REPEATS, "none");
    let every = input.int("repeatEvery", 1, 365, 1);
    let until = input.str("repeatUntil", DATE).into_option();
    input.strict()?;
    if source.is_some() && asset.is_none() {
        return Err(AppError::new(400, "Choose the item whose date this reminder follows"));
    }
    if kind == "custom" && title.is_none() {
        return Err(AppError::new(400, "Give a custom reminder a title"));
    }
    let mut tx = state.pool.begin().await?;
    let asset_name: Option<String> = match asset {
        Some(a) => {
            user.require("assets:read")?;
            let name: Option<String> = Q::new(format!(
                "SELECT a.name FROM assets a WHERE a.org_id = $1 AND a.id = $2 AND {}",
                user.scope_sql("a", 3)
            ))
            .bind(user.org_id)
            .bind(a)
            .bind(user.scope_param())
            .scalars(&mut tx)
            .await?
            .into_iter()
            .next();
            Some(name.ok_or_else(|| AppError::new(400, "Item not found"))?)
        }
        None => None,
    };
    let due = match (&source, asset, due) {
        (Some(s), Some(a), _) => source_date(&mut tx, user.org_id, a, s).await?.ok_or_else(|| {
            AppError::new(
                400,
                "That item has no date for this yet. Add the date to the item first, or enter a due date.",
            )
        })?,
        (None, _, Some(d)) => d,
        _ => return Err(AppError::new(400, "Enter the due date")),
    };
    if repeat != "none" && source.is_some() {
        return Err(AppError::new(400, "A reminder that follows an item's date cannot also repeat"));
    }
    let offs = match offs {
        Some(o) => o,
        None => Q::new("SELECT default_offsets FROM notification_settings WHERE user_id = $1")
            .bind(user.id)
            .scalars::<Vec<i32>>(&mut tx)
            .await?
            .into_iter()
            .next()
            .unwrap_or_else(|| vec![7, 1]),
    };
    let title = title.unwrap_or_else(|| default_title(&kind, asset_name.as_deref()));
    crate::routes::plans::check_limit(&mut tx, user.org_id, "reminders").await?;
    let id: Uuid = Q::new(
        "INSERT INTO user_reminders (org_id, user_id, asset_id, kind, title, notes, date_source, due_on, offsets,
                                     repeat, repeat_every, repeat_until)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::date, $9, $10, $11, $12::date) RETURNING id",
    )
    .bind(user.org_id)
    .bind(user.id)
    .bind(asset)
    .bind(&kind)
    .bind(&title)
    .bind(notes)
    .bind(source)
    .bind(due)
    .bind(offs)
    .bind(&repeat)
    .bind(every as i32)
    .bind(until)
    .scalar(&mut tx)
    .await?;
    let created = find(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// Edits or reschedules a reminder, pauses or resumes it.
pub async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let title = input.str("title", Str::req(1, 160).optional()).into_option();
    let notes = input.str("notes", Str::text(1000));
    let due = input.str("dueOn", DATE);
    let offs = offsets(&mut input, true);
    let repeat = input.one_of("repeat", REPEATS, true, false).into_option();
    let every = input.raw_value("repeatEvery").map(|_| ());
    let every = every.map(|_| input.int("repeatEvery", 1, 365, 1) as i32);
    let until = input.str("repeatUntil", DATE);
    let status = input.one_of("status", &["active", "paused"], true, false).into_option();
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let current = find(&mut tx, &user, id).await?;
    if current["status"] == "completed" && status.is_none() {
        return Err(AppError::new(409, "This reminder is complete. Set it active again to change it."));
    }
    // Choosing a date by hand stops following the item's date.
    let unfollow = matches!(due, Opt::Val(_));
    if matches!(due, Opt::Null) {
        return Err(AppError::new(400, "A reminder needs a due date"));
    }
    let repeating = repeat.as_deref().unwrap_or(current["repeat"].as_str().unwrap_or("none")) != "none";
    if repeating && !current["dateSource"].is_null() && !unfollow {
        return Err(AppError::new(400, "A reminder that follows an item's date cannot also repeat"));
    }
    Q::new(
        "UPDATE user_reminders SET
           title = COALESCE($3, title),
           notes = CASE WHEN $4 THEN $5 ELSE notes END,
           due_on = COALESCE($6::date, due_on),
           date_source = CASE WHEN $6::date IS NOT NULL THEN NULL ELSE date_source END,
           offsets = COALESCE($7, offsets),
           repeat = COALESCE($8, repeat),
           repeat_every = COALESCE($9, repeat_every),
           repeat_until = CASE WHEN $10 THEN $11::date ELSE repeat_until END,
           status = COALESCE($12, status),
           completed_at = CASE WHEN $12 IS NOT NULL THEN NULL ELSE completed_at END,
           snoozed_until = CASE WHEN $6::date IS NOT NULL OR $12 IS NOT NULL THEN NULL ELSE snoozed_until END,
           updated_at = now()
         WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .bind(title)
    .bind(notes.is_present())
    .bind(notes.into_option())
    .bind(due.into_option())
    .bind(offs)
    .bind(repeat)
    .bind(every)
    .bind(until.is_present())
    .bind(until.into_option())
    .bind(status)
    .run(&mut tx)
    .await?;
    let updated = find(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok(Json(updated))
}

/// Puts a reminder off: no notifications until then, then one reminder at that time.
pub async fn snooze(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let hours = input.raw_value("hours").map(|_| ());
    let hours = hours.map(|_| input.int("hours", 1, 24 * 90, 24));
    let until = input.str("until", Str { trim: false, ..Str::req(0, 100).optional() }.check(is_datetime)).into_option();
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let current = find(&mut tx, &user, id).await?;
    if current["status"] != "active" {
        return Err(AppError::new(409, "Only an active reminder can be snoozed"));
    }
    let n = Q::new(
        "UPDATE user_reminders SET snoozed_until = COALESCE($3::timestamptz, now() + make_interval(hours => $4)), updated_at = now()
          WHERE id = $1 AND user_id = $2
            AND COALESCE($3::timestamptz, now() + make_interval(hours => $4)) > now()
            AND COALESCE($3::timestamptz, now() + make_interval(hours => $4)) < now() + interval '366 days'",
    )
    .bind(id)
    .bind(user.id)
    .bind(until)
    .bind(hours.unwrap_or(24) as i32)
    .run(&mut tx)
    .await?;
    if n == 0 {
        return Err(AppError::new(400, "Snooze until a time in the next year"));
    }
    let updated = find(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok(Json(updated))
}

fn is_datetime(s: &str) -> Option<&'static str> {
    chrono::DateTime::parse_from_rfc3339(s).is_err().then_some("Invalid datetime")
}

/// Marks a reminder done. A recurring reminder moves on to its next date instead.
pub async fn complete(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    let id = path_uuid(&id)?;
    let mut tx = state.pool.begin().await?;
    let current = find(&mut tx, &user, id).await?;
    if current["status"] == "completed" {
        return Ok(Json(current));
    }
    let repeat = current["repeat"].as_str().unwrap_or("none");
    let due = current["dueOn"].as_str().and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    let next = match (repeat, due) {
        ("none", _) | (_, None) => None,
        (r, Some(d)) => {
            let today = d.max(chrono::Utc::now().date_naive());
            next_due(d, r, current["repeatEvery"].as_u64().unwrap_or(1) as u32, today.succ_opt().unwrap_or(today))
        }
    };
    let until = current["repeatUntil"].as_str().and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    match next.filter(|n| until.is_none_or(|u| *n <= u)) {
        Some(n) => Q::new(
            "UPDATE user_reminders SET due_on = $3::date, snoozed_until = NULL, updated_at = now() WHERE id = $1 AND user_id = $2",
        )
        .bind(id)
        .bind(user.id)
        .bind(n.to_string()),
        None => Q::new(
            "UPDATE user_reminders SET status = 'completed', completed_at = now(), snoozed_until = NULL, updated_at = now()
              WHERE id = $1 AND user_id = $2",
        )
        .bind(id)
        .bind(user.id),
    }
    .run(&mut tx)
    .await?;
    let updated = find(&mut tx, &user, id).await?;
    tx.commit().await?;
    Ok(Json(updated))
}

pub async fn delete(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<StatusCode> {
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let n = Q::new("DELETE FROM user_reminders WHERE id = $1 AND user_id = $2 AND org_id = $3")
        .bind(id)
        .bind(user.id)
        .bind(user.org_id)
        .run(&mut db)
        .await?;
    if n == 0 {
        return Err(AppError::not_found("Reminder"));
    }
    Ok(StatusCode::NO_CONTENT)
}
