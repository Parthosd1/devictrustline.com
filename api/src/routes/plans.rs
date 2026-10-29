//! Plans and their limits, enforced here on the server. Online payment is a boundary only: until
//! a billing provider is connected, checkout says so and the operator changes plans with
//! `devicetrustline set-workspace-plan`.

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::User;
use crate::db::{Db, Q};
use crate::error::{AppError, Result};
use crate::validate::{Input, Str};
use crate::web::Body;
use crate::AppState;

pub const LIMITS: &[(&str, &str)] = &[
    ("items", "items"),
    ("members", "people"),
    ("devices", "paired computers"),
    ("storageMb", "MB of files"),
    ("inventories", "inventories"),
    ("reminders", "active reminders"),
];
/// Switches a plan includes or not.
pub const FEATURES: &[(&str, &str)] = &[("sms", "Text message notifications"), ("reports", "Reports")];

/// What the workspace uses of each limited resource right now.
async fn usage(db: &mut Db, org: Uuid) -> Result<Value> {
    Q::new(
        r#"SELECT (SELECT count(*)::int FROM assets WHERE org_id = $1 AND status <> 'Retired') AS items,
                  (SELECT count(*)::int FROM memberships WHERE org_id = $1 AND is_active) AS members,
                  (SELECT count(*)::int FROM device_agents WHERE org_id = $1 AND revoked_at IS NULL) AS devices,
                  (SELECT COALESCE(ceil(sum(size_bytes) / 1048576.0), 0)::int FROM attachments WHERE org_id = $1) AS "storageMb",
                  (SELECT count(*)::int FROM inventories WHERE org_id = $1 AND archived_at IS NULL) AS inventories,
                  (SELECT count(*)::int FROM user_reminders WHERE org_id = $1 AND status <> 'completed') AS reminders"#,
    )
    .bind(org)
    .one(db)
    .await
}

async fn plan_of(db: &mut Db, org: Uuid) -> Result<Value> {
    Q::new(
        "SELECT p.code, p.name, p.kind, p.limits, p.features
           FROM organizations o JOIN plan_definitions p ON p.code = o.plan WHERE o.id = $1",
    )
    .bind(org)
    .one(db)
    .await
}

fn refuse(plan: &Value, key: &str, limit: i64) -> AppError {
    let noun = LIMITS.iter().find(|(k, _)| *k == key).map(|(_, n)| *n).unwrap_or(key);
    AppError::coded(
        402,
        "plan_limit",
        format!(
            "The {} plan includes up to {limit} {noun}. Remove some, or move to a larger plan under Plan & billing.",
            plan["name"].as_str().unwrap_or("current")
        ),
    )
}

/// Refuses adding `more` of a limited resource when that would go over the plan's limit.
/// Locks the workspace row, so concurrent requests cannot both squeeze under the limit.
pub async fn check_limit_by(db: &mut Db, org: Uuid, key: &str, more: i64) -> Result<()> {
    Q::new("SELECT 1 FROM organizations WHERE id = $1 FOR UPDATE").bind(org).run(&mut *db).await?;
    let plan = plan_of(db, org).await?;
    let Some(limit) = plan["limits"][key].as_i64() else { return Ok(()) };
    let used = usage(db, org).await?[key].as_i64().unwrap_or(0);
    if used + more > limit {
        return Err(refuse(&plan, key, limit));
    }
    Ok(())
}

pub async fn check_limit(db: &mut Db, org: Uuid, key: &str) -> Result<()> {
    check_limit_by(db, org, key, 1).await
}

/// Refuses something the workspace's plan does not include.
pub async fn require_feature(db: &mut Db, org: Uuid, feature: &str) -> Result<()> {
    let plan = plan_of(db, org).await?;
    if plan["features"][feature] == true {
        return Ok(());
    }
    let what = FEATURES.iter().find(|(k, _)| *k == feature).map(|(_, n)| *n).unwrap_or(feature);
    Err(AppError::coded(
        402,
        "plan_feature",
        format!(
            "{what} are not part of the {} plan. A larger plan includes them; see Plan & billing.",
            plan["name"].as_str().unwrap_or("current")
        ),
    ))
}

pub async fn check_storage(db: &mut Db, org: Uuid, bytes: i64) -> Result<()> {
    Q::new("SELECT 1 FROM organizations WHERE id = $1 FOR UPDATE").bind(org).run(&mut *db).await?;
    let plan = plan_of(db, org).await?;
    let Some(limit_mb) = plan["limits"]["storageMb"].as_i64() else { return Ok(()) };
    let used: i64 = Q::new("SELECT COALESCE(sum(size_bytes), 0)::bigint FROM attachments WHERE org_id = $1")
        .bind(org)
        .scalar(&mut *db)
        .await?;
    if used + bytes > limit_mb * 1024 * 1024 {
        return Err(refuse(&plan, "storageMb", limit_mb));
    }
    Ok(())
}

pub fn billing_configured() -> bool {
    false
}

pub async fn get(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    let mut db = state.pool.acquire().await?;
    let plan = plan_of(&mut db, user.org_id).await?;
    let used = usage(&mut db, user.org_id).await?;
    let plans = Q::new("SELECT code, name, limits, features FROM plan_definitions WHERE kind = $1 ORDER BY position")
        .bind(plan["kind"].as_str())
        .all(&mut db)
        .await?;
    Ok(Json(json!({
        "plan": plan,
        "usage": used,
        "plans": plans,
        "billing": { "configured": billing_configured() },
    })))
}

/// The billing boundary. No payment provider is connected in this release, so this explains how
/// a plan change happens instead of pretending to take payment.
pub async fn checkout(user: User, Body(body): Body) -> Result<Json<Value>> {
    user.require("settings:manage")?;
    let mut input = Input::body(&body);
    input.req("plan", Str::req(1, 40));
    input.strict()?;
    Err(AppError::coded(
        503,
        "billing_unconfigured",
        "Online payment is not set up on this server yet. Ask the service operator to change your plan.",
    ))
}
