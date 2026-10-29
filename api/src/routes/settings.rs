use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::auth::User;
use crate::db::Q;
use crate::error::{AppError, Result};
use crate::reminders::DEFAULT_DAYS;
use crate::validate::{Input, Str};
use crate::web::Body;
use crate::AppState;

/// An organization's settings with every default filled in.
pub fn with_defaults(settings: &Value) -> Value {
    let mut reminders = json!({ "expiringSoonDays": 30 });
    for (_, key, days) in DEFAULT_DAYS {
        reminders[*key] = json!(days);
    }
    let mut notifications = json!({});
    for (group, _, _) in crate::notify::GROUPS {
        notifications[*group] = crate::notify::Channels::ALL.to_json();
    }
    let mut out = json!({
        "autoAudits": { "weekly": false, "monthly": false, "quarterly": false, "annual": false },
        "reminders": reminders, "currency": null, "notifications": notifications,
    });
    if let Some(map) = settings.as_object() {
        for (k, v) in map {
            match (out.get_mut(k), v.as_object()) {
                // Nested groups keep defaults for keys not set.
                (Some(Value::Object(group)), Some(set)) => {
                    for (gk, gv) in set {
                        group.insert(gk.clone(), gv.clone());
                    }
                }
                _ => out[k] = v.clone(),
            }
        }
    }
    out
}

pub async fn get(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    let mut db = state.pool.acquire().await?;
    let row =
        Q::new("SELECT settings, timezone FROM organizations WHERE id = $1").bind(user.org_id).one(&mut db).await?;
    let mut out = with_defaults(&row["settings"]);
    out["timezone"] = row["timezone"].clone();
    Ok(Json(out))
}

fn day_list(input: &mut Input, key: &str) -> Option<Vec<i64>> {
    let raw = input.raw_value(key)?;
    let list: Option<Vec<i64>> =
        raw.as_array().and_then(|a| a.iter().map(|v| v.as_i64().filter(|n| (0..=730).contains(n))).collect());
    match list {
        Some(mut l) if l.len() <= 6 => {
            l.sort_unstable_by(|a, b| b.cmp(a));
            l.dedup();
            Some(l)
        }
        _ => {
            input.fail(key, "List up to 6 whole numbers of days, from 0 to 730");
            None
        }
    }
}

pub async fn update(State(state): State<AppState>, user: User, Body(body): Body) -> Result<Json<Value>> {
    user.require("settings:manage")?;
    let mut input = Input::body(&body);
    let mut auto = vec![];
    input.object("autoAudits", |a| {
        for period in ["weekly", "monthly", "quarterly", "annual"] {
            if let Some(on) = a.boolean(period, false).into_option() {
                auto.push((period, on));
            }
        }
    });
    let mut reminders = serde_json::Map::new();
    input.object("reminders", |r| {
        for (_, key, _) in DEFAULT_DAYS {
            if let Some(days) = day_list(r, key) {
                reminders.insert(key.to_string(), json!(days));
            }
        }
        if let Some(raw) = r.raw_value("expiringSoonDays") {
            match raw.as_i64().filter(|n| (1..=365).contains(n)) {
                Some(n) => {
                    reminders.insert("expiringSoonDays".into(), json!(n));
                }
                None => r.fail("expiringSoonDays", "A whole number of days from 1 to 365"),
            }
        }
    });
    // Workspace defaults for event notifications, by group.
    let mut notifications = vec![];
    if let Some(raw) = input.raw_value("notifications") {
        match raw.as_object() {
            Some(map) => {
                for (group, v) in map {
                    match (
                        crate::notify::GROUPS.iter().any(|(g, _, _)| g == group),
                        crate::notify::Channels::from_json(v),
                    ) {
                        (true, Some(c)) => notifications.push((group.clone(), c)),
                        (false, _) => input.fail(&format!("notifications.{group}"), "Unknown notification group"),
                        (true, None) => input.fail(
                            &format!("notifications.{group}"),
                            "Give inApp, email, push and sms as true or false",
                        ),
                    }
                }
            }
            None => input.fail("notifications", "Must be an object of groups"),
        }
    }
    let timezone = input.str("timezone", Str::req(1, 64).optional()).into_option();
    let currency = input.str("currency", Str::text(3)).into_option();
    if currency.as_ref().is_some_and(|c| c.len() != 3 || !c.chars().all(|ch| ch.is_ascii_alphabetic())) {
        input.fail("currency", "Use a 3-letter currency code, like USD");
    }
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    if let Some(tz) = &timezone {
        let known: i64 =
            Q::new("SELECT count(*) FROM pg_timezone_names WHERE name = $1").bind(tz).scalar(&mut tx).await?;
        if known == 0 {
            return Err(AppError::Invalid(vec![crate::validate::Issue::new(
                "timezone",
                "Unknown time zone. Use a name like America/New_York",
            )]));
        }
        Q::new("UPDATE organizations SET timezone = $2 WHERE id = $1").bind(user.org_id).bind(tz).run(&mut tx).await?;
    }
    let settings: Value =
        Q::new("SELECT settings FROM organizations WHERE id = $1 FOR UPDATE").bind(user.org_id).scalar(&mut tx).await?;
    let mut next = with_defaults(&settings);
    for (period, on) in auto {
        next["autoAudits"][period] = json!(on);
    }
    for (k, v) in reminders {
        next["reminders"][k] = v;
    }
    for (group, c) in notifications {
        next["notifications"][group] = c.to_json();
    }
    if let Some(c) = currency {
        next["currency"] = json!(c.to_uppercase());
    }
    Q::new("UPDATE organizations SET settings = $2 WHERE id = $1").bind(user.org_id).bind(&next).run(&mut tx).await?;
    let tz: String =
        Q::new("SELECT timezone FROM organizations WHERE id = $1").bind(user.org_id).scalar(&mut tx).await?;
    tx.commit().await?;
    next["timezone"] = json!(tz);
    Ok(Json(next))
}
