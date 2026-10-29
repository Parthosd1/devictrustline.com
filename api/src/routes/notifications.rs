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
use crate::validate::{Input, Opt};
use crate::web::Body;
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
                  n.read_at AS "readAt", CASE WHEN e.status = 'sending' THEN 'queued' ELSE e.status END AS "emailStatus",
                  n.link, n.user_reminder_id AS "reminderId", ({}) AS deliveries
             FROM notifications n LEFT JOIN email_outbox e ON e.id = n.email_id
            WHERE n.user_id = $1 AND n.org_id = $2 AND ($3 = false OR n.read_at IS NULL)
            ORDER BY n.created_at DESC, n.id DESC LIMIT {limit}"#,
        crate::notify::DELIVERIES_SQL
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
              AND ($2::uuid[] IS NULL OR (SELECT inventory_id FROM assets WHERE id = d.asset_id) = ANY($2))
            ORDER BY d.due_on, d.title LIMIT 200"#
    ))
    .bind(user.org_id)
    .bind(user.scope_param())
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

const SETTINGS_SELECT: &str = r#"
  SELECT COALESCE(s.enabled, true) AS enabled, true AS "inApp", u.email_notifications AS email,
         COALESCE(s.push, false) AS push, COALESCE(s.sms, false) AS sms, s.phone,
         s.timezone, o.timezone AS "workspaceTimezone",
         to_char(s.quiet_start, 'HH24:MI') AS "quietStart", to_char(s.quiet_end, 'HH24:MI') AS "quietEnd",
         COALESCE(s.send_hour, 9) AS "sendHour", COALESCE(s.default_offsets, '{7,1}') AS "defaultOffsets",
         (SELECT count(*)::int FROM push_subscriptions p WHERE p.user_id = u.id) AS "pushSubscriptions"
    FROM users u JOIN organizations o ON o.id = $2
    LEFT JOIN notification_settings s ON s.user_id = u.id
   WHERE u.id = $1"#;

async fn settings_of(db: &mut crate::db::Db, state: &AppState, user: &User) -> Result<Value> {
    let mut row = Q::new(SETTINGS_SELECT).bind(user.id).bind(user.org_id).one(db).await?;
    // What this server can actually deliver, so the app never offers a channel that cannot work.
    row["available"] = json!({
        "email": state.config.mail.available(),
        "push": state.config.push.is_some(),
        "sms": state.config.sms.available(),
    });
    row["pushPublicKey"] = json!(state.config.push.as_ref().map(|p| p.public_key.clone()));
    Ok(row)
}

/// The signed-in person's notification preferences. They apply in every workspace.
pub async fn get_settings(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    let mut db = state.pool.acquire().await?;
    Ok(Json(settings_of(&mut db, &state, &user).await?))
}

fn time_check(s: &str) -> Option<&'static str> {
    chrono::NaiveTime::parse_from_str(s, "%H:%M").is_err().then_some("Use HH:MM, 24-hour")
}

fn phone_check(s: &str) -> Option<&'static str> {
    let ok = s.starts_with('+')
        && (8..=16).contains(&s.len())
        && s[1..].bytes().all(|b| b.is_ascii_digit())
        && !s[1..].starts_with('0');
    (!ok).then_some("Use the international format, like +14155550123")
}

pub async fn update_settings(State(state): State<AppState>, user: User, Body(body): Body) -> Result<Json<Value>> {
    let mut input = Input::body(&body);
    let enabled = input.boolean("enabled", false).into_option();
    let email = input.boolean("email", false).into_option();
    let push = input.boolean("push", false).into_option();
    let sms = input.boolean("sms", false).into_option();
    let phone = input.str("phone", crate::validate::Str::text(20).check(phone_check));
    let timezone = input.str("timezone", crate::validate::Str::text(64));
    let quiet_start = input.str("quietStart", crate::validate::Str::text(5).check(time_check));
    let quiet_end = input.str("quietEnd", crate::validate::Str::text(5).check(time_check));
    let send_hour = input.raw_value("sendHour").map(|_| ());
    let send_hour = send_hour.map(|_| input.int("sendHour", 0, 23, 9) as i16);
    let offsets = input.raw_value("defaultOffsets").and_then(|raw| {
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
                input.fail("defaultOffsets", "List 1 to 8 whole numbers of days, from 0 to 365");
                None
            }
        }
    });
    if quiet_start.is_present() != quiet_end.is_present()
        || quiet_start.value().is_some() != quiet_end.value().is_some()
    {
        input.fail("quietEnd", "Set both the start and end of quiet hours, or clear both");
    }
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    if let Opt::Val(tz) = &timezone {
        let known: i64 =
            Q::new("SELECT count(*) FROM pg_timezone_names WHERE name = $1").bind(tz).scalar(&mut tx).await?;
        if known == 0 {
            return Err(AppError::Invalid(vec![crate::validate::Issue::new(
                "timezone",
                "Unknown time zone. Use a name like America/New_York",
            )]));
        }
    }
    if let Some(email) = email {
        Q::new("UPDATE users SET email_notifications = $2, updated_at = now() WHERE id = $1")
            .bind(user.id)
            .bind(email)
            .run(&mut tx)
            .await?;
    }
    Q::new("INSERT INTO notification_settings (user_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(user.id)
        .run(&mut tx)
        .await?;
    Q::new(
        "UPDATE notification_settings SET
           enabled = COALESCE($2, enabled), push = COALESCE($3, push), sms = COALESCE($4, sms),
           phone = CASE WHEN $5 THEN $6 ELSE phone END,
           timezone = CASE WHEN $7 THEN $8 ELSE timezone END,
           quiet_start = CASE WHEN $9 THEN $10::time ELSE quiet_start END,
           quiet_end = CASE WHEN $9 THEN $11::time ELSE quiet_end END,
           send_hour = COALESCE($12, send_hour), default_offsets = COALESCE($13, default_offsets), updated_at = now()
         WHERE user_id = $1",
    )
    .bind(user.id)
    .bind(enabled)
    .bind(push)
    .bind(sms)
    .bind(phone.is_present())
    .bind(phone.into_option())
    .bind(timezone.is_present())
    .bind(timezone.into_option())
    .bind(quiet_start.is_present())
    .bind(quiet_start.into_option())
    .bind(quiet_end.into_option())
    .bind(send_hour)
    .bind(offsets)
    .run(&mut tx)
    .await?;
    let row: Value =
        Q::new("SELECT sms, phone FROM notification_settings WHERE user_id = $1").bind(user.id).one(&mut tx).await?;
    if row["sms"] == true && row["phone"].is_null() {
        return Err(AppError::Invalid(vec![crate::validate::Issue::new(
            "phone",
            "Add a phone number to get text messages",
        )]));
    }
    let out = settings_of(&mut tx, &state, &user).await?;
    tx.commit().await?;
    Ok(Json(out))
}

/// Registers this browser for push notifications (after the person allowed them in the browser).
pub async fn subscribe_push(
    State(state): State<AppState>,
    user: User,
    headers: axum::http::HeaderMap,
    Body(body): Body,
) -> Result<StatusCode> {
    if state.config.push.is_none() {
        return Err(AppError::coded(
            503,
            "push_unconfigured",
            "Browser notifications are not set up on this server yet.",
        ));
    }
    let mut input = Input::body(&body);
    let endpoint = input.req("endpoint", crate::validate::Str::req(10, 1000));
    let mut keys = (String::new(), String::new());
    input.object("keys", |k| {
        keys = (k.req("p256dh", crate::validate::Str::req(20, 200)), k.req("auth", crate::validate::Str::req(10, 100)));
    });
    input.raw_value("expirationTime");
    input.strict()?;
    if !endpoint.starts_with("https://") && !(endpoint.starts_with("http://127.0.0.1") && !state.config.is_production) {
        return Err(AppError::new(400, "A push endpoint must be an https:// address"));
    }
    // Refuse keys that cannot be used, rather than failing on every later delivery.
    crate::push::encrypt(&keys.0, &keys.1, b"check")
        .map_err(|e| AppError::new(400, format!("Invalid subscription: {e}")))?;
    let agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().take(200).collect::<String>());
    let mut tx = state.pool.begin().await?;
    Q::new(
        "INSERT INTO push_subscriptions (user_id, endpoint, p256dh, auth, user_agent) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (endpoint) DO UPDATE SET user_id = EXCLUDED.user_id, p256dh = EXCLUDED.p256dh, auth = EXCLUDED.auth,
                                              user_agent = EXCLUDED.user_agent",
    )
    .bind(user.id)
    .bind(&endpoint)
    .bind(&keys.0)
    .bind(&keys.1)
    .bind(agent)
    .run(&mut tx)
    .await?;
    Q::new("INSERT INTO notification_settings (user_id, push) VALUES ($1, true) ON CONFLICT (user_id) DO UPDATE SET push = true")
        .bind(user.id)
        .run(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::CREATED)
}

pub async fn unsubscribe_push(State(state): State<AppState>, user: User, Body(body): Body) -> Result<StatusCode> {
    let mut input = Input::body(&body);
    let endpoint = input.str("endpoint", crate::validate::Str::req(1, 1000).optional()).into_option();
    input.strict()?;
    let mut db = state.pool.acquire().await?;
    // Without an endpoint, every browser of this person is forgotten.
    Q::new("DELETE FROM push_subscriptions WHERE user_id = $1 AND ($2::text IS NULL OR endpoint = $2)")
        .bind(user.id)
        .bind(endpoint)
        .run(&mut db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sends a test notification to the signed-in person over every channel they turned on.
pub async fn send_test(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    let mut tx = state.pool.begin().await?;
    let key = format!("test:{}:{}", user.id, crate::tokens::new_token());
    let notice = crate::notify::Notice {
        dedupe_key: Some(key),
        ..crate::notify::Notice::new(
            user.org_id,
            user.id,
            "test",
            "Test notification",
            "This is a test from DeviceTrustline. If you can read it here, this channel works.",
        )
    };
    let id = crate::notify::send(&mut tx, &state.config, notice).await?;
    tx.commit().await?;
    let Some(id) = id else {
        return Err(AppError::new(409, "Notifications are turned off in your notification settings"));
    };
    Ok(Json(json!({ "id": id })))
}

pub async fn delete(State(state): State<AppState>, user: User, Path(id): Path<i64>) -> Result<StatusCode> {
    let mut db = state.pool.acquire().await?;
    let n = Q::new("DELETE FROM notifications WHERE id = $1 AND user_id = $2 AND org_id = $3")
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

/// Workspace event notifications by group: the workspace default, this person's own choice (if
/// any) and what applies.
pub async fn get_event_preferences(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    let mut db = state.pool.acquire().await?;
    let mut groups = vec![];
    for (group, label, kinds) in crate::notify::GROUPS {
        let row = Q::new(
            r#"SELECT (SELECT json_build_object('inApp', p.in_app, 'email', p.email, 'push', p.push, 'sms', p.sms)
                         FROM notification_preferences p WHERE p.user_id = $2 AND p.org_id = $1 AND p.grp = $3) AS own,
                      (SELECT o.settings->'notifications'->$3 FROM organizations o WHERE o.id = $1) AS workspace"#,
        )
        .bind(user.org_id)
        .bind(user.id)
        .bind(*group)
        .one(&mut db)
        .await?;
        let workspace = crate::notify::Channels::from_json(&row["workspace"]).unwrap_or(crate::notify::Channels::ALL);
        let own = crate::notify::Channels::from_json(&row["own"]);
        groups.push(json!({
            "group": group, "label": label, "kinds": kinds,
            "workspace": workspace.to_json(), "own": own.map(|c| c.to_json()),
            "effective": own.unwrap_or(workspace).to_json(),
        }));
    }
    Ok(Json(json!({ "groups": groups })))
}

/// Sets this person's choices: `{ "<group>": {inApp, email, push, sms} | null }`; null returns
/// the group to the workspace default.
pub async fn update_event_preferences(
    State(state): State<AppState>,
    user: User,
    Body(body): Body,
) -> Result<Json<Value>> {
    let Some(map) = body.as_object() else {
        return Err(AppError::new(400, "Send an object of groups"));
    };
    let mut changes = vec![];
    for (group, value) in map {
        if crate::notify::GROUPS.iter().all(|(g, _, _)| g != group) {
            return Err(AppError::Invalid(vec![crate::validate::Issue::new(group, "Unknown notification group")]));
        }
        let channels = match value {
            Value::Null => None,
            v => Some(crate::notify::Channels::from_json(v).ok_or_else(|| {
                AppError::Invalid(vec![crate::validate::Issue::new(
                    group,
                    "Give inApp, email, push and sms as true or false",
                )])
            })?),
        };
        changes.push((group.clone(), channels));
    }
    let mut tx = state.pool.begin().await?;
    for (group, channels) in changes {
        match channels {
            None => {
                Q::new("DELETE FROM notification_preferences WHERE user_id = $1 AND org_id = $2 AND grp = $3")
                    .bind(user.id)
                    .bind(user.org_id)
                    .bind(&group)
                    .run(&mut tx)
                    .await?;
            }
            Some(c) => {
                Q::new(
                    "INSERT INTO notification_preferences (user_id, org_id, grp, in_app, email, push, sms)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT (user_id, org_id, grp) DO UPDATE SET in_app = $4, email = $5, push = $6, sms = $7, updated_at = now()",
                )
                .bind(user.id)
                .bind(user.org_id)
                .bind(&group)
                .bind(c.in_app)
                .bind(c.email)
                .bind(c.push)
                .bind(c.sms)
                .run(&mut tx)
                .await?;
            }
        }
    }
    tx.commit().await?;
    get_event_preferences(State(state), user).await
}
