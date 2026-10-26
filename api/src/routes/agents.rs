//! The device companion: pairing computers to a workspace, receiving what they find, and the
//! review queue where people add, link or ignore discovered devices.
//!
//! An agent authenticates with `Authorization: Bearer dta_…`, a credential that only reaches the
//! agent endpoints of its own workspace and stops working the moment it is revoked.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, Query, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::auth::User;
use crate::db::{uuid_of, Db, Q};
use crate::error::{AppError, Result};
use crate::routes::assets::{find_asset, record_event};
use crate::tokens;
use crate::validate::{path_uuid, Input, Str};
use crate::web::Body;
use crate::AppState;

const TOKEN_PREFIX: &str = "dta_";
pub const CLASSES: &[&str] =
    &["computer", "monitor", "dock", "printer", "storage", "network", "peripheral", "phone", "other"];

/// Serial numbers that firmware reports when it has none. They identify nothing.
pub fn real_serial(serial: &str) -> Option<String> {
    let s = serial.trim();
    let lower = s.to_lowercase();
    let junk = [
        "to be filled by o.e.m.",
        "default string",
        "system serial number",
        "not specified",
        "none",
        "n/a",
        "na",
        "unknown",
        "0",
        "00000000",
        "0123456789",
        "123456789",
        "1234567890",
        "serial",
        "invalid",
        "chassis serial number",
    ];
    let one_char = s.chars().all(|c| s.starts_with(c));
    if s.len() < 3 || junk.contains(&lower.as_str()) || one_char {
        None
    } else {
        Some(s.to_string())
    }
}

/// How a device is recognized across reports and computers.
pub fn fingerprint(
    agent: Uuid,
    class: &str,
    manufacturer: Option<&str>,
    serial: Option<&str>,
    local_id: &str,
) -> String {
    match serial.and_then(real_serial) {
        Some(sn) => format!("sn:{}:{class}:{}", manufacturer.unwrap_or("").trim().to_lowercase(), sn.to_uppercase()),
        None => format!("local:{agent}:{}", local_id.trim().to_lowercase()),
    }
}

// ---- Workspace side ------------------------------------------------------------------------

pub async fn create_pairing_code(State(state): State<AppState>, user: User) -> Result<(StatusCode, Json<Value>)> {
    user.require("devices:manage")?;
    let mut tx = state.pool.begin().await?;
    crate::routes::plans::check_limit(&mut tx, user.org_id, "devices").await?;
    let code = tokens::short_code(10);
    let row = Q::new(
        "INSERT INTO agent_pairing_codes (code_hash, org_id, created_by, expires_at)
         VALUES ($1, $2, $3, now() + interval '15 minutes') RETURNING expires_at",
    )
    .bind(tokens::hash(&code))
    .bind(user.org_id)
    .bind(user.id)
    .one(&mut tx)
    .await?;
    tx.commit().await?;
    let display = format!("{}-{}", &code[..5], &code[5..]);
    Ok((
        StatusCode::CREATED,
        Json(json!({ "code": display, "expiresAt": row["expires_at"], "server": state.config.app_url })),
    ))
}

const AGENT_SELECT: &str = r#"SELECT g.id, g.name, g.platform, g.agent_version AS "version", g.paused,
       g.created_at AS "createdAt", g.last_seen_at AS "lastSeenAt", g.revoked_at AS "revokedAt",
       (SELECT count(*)::int FROM discovered_devices d WHERE d.agent_id = g.id AND d.state = 'new') AS "newDevices",
       (SELECT count(*)::int FROM discovered_devices d WHERE d.agent_id = g.id) AS "devices"
  FROM device_agents g"#;

pub async fn list_agents(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!("{AGENT_SELECT} WHERE g.org_id = $1 ORDER BY g.revoked_at NULLS FIRST, g.name"))
        .bind(user.org_id)
        .all(&mut db)
        .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn update_agent(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("devices:manage")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let name = input.str("name", Str::req(1, 120).optional()).into_option();
    let paused = input.boolean("paused", false).into_option();
    input.strict()?;
    let mut db = state.pool.acquire().await?;
    let n = Q::new(
        "UPDATE device_agents SET name = COALESCE($3, name), paused = COALESCE($4, paused)
          WHERE org_id = $1 AND id = $2 AND revoked_at IS NULL",
    )
    .bind(user.org_id)
    .bind(id)
    .bind(name)
    .bind(paused)
    .run(&mut db)
    .await?;
    if n == 0 {
        return Err(AppError::not_found("Computer"));
    }
    Ok(Json(Q::new(format!("{AGENT_SELECT} WHERE g.id = $1")).bind(id).one(&mut db).await?))
}

/// Disconnects a computer: its credential stops working immediately. What it found stays.
pub async fn revoke_agent(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<StatusCode> {
    user.require("devices:manage")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let n = Q::new("UPDATE device_agents SET revoked_at = now() WHERE org_id = $1 AND id = $2 AND revoked_at IS NULL")
        .bind(user.org_id)
        .bind(id)
        .run(&mut db)
        .await?;
    if n == 0 {
        return Err(AppError::not_found("Computer"));
    }
    Ok(StatusCode::NO_CONTENT)
}

const DEVICE_SELECT: &str = r#"SELECT d.id, d.agent_id AS "agentId", g.name AS "agentName", d.class, d.name, d.manufacturer,
       d.model, d.serial, d.details, d.state, d.asset_id AS "assetId", a.asset_tag AS "assetTag", a.name AS "assetName",
       d.first_seen_at AS "firstSeenAt", d.last_seen_at AS "lastSeenAt", d.missing_since AS "missingSince",
       d.fingerprint LIKE 'sn:%' AS "identifiedBySerial"
  FROM discovered_devices d JOIN device_agents g ON g.id = d.agent_id LEFT JOIN assets a ON a.id = d.asset_id"#;

pub async fn list_devices(
    State(state): State<AppState>,
    user: User,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let mut input = Input::query(&q);
    let filter = input.one_of_or("state", &["new", "linked", "ignored", "all"], "new");
    let agent = input.uuid("agentId", true, false).into_option();
    input.done()?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!(
        "{DEVICE_SELECT} WHERE d.org_id = $1 AND ($2 = 'all' OR d.state = $2) AND ($3::uuid IS NULL OR d.agent_id = $3)
          ORDER BY d.missing_since NULLS FIRST, d.class, d.name LIMIT 500"
    ))
    .bind(user.org_id)
    .bind(filter)
    .bind(agent)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

async fn find_device(db: &mut Db, org: Uuid, id: Uuid) -> Result<Value> {
    Q::new(format!("{DEVICE_SELECT} WHERE d.org_id = $1 AND d.id = $2 FOR UPDATE OF d"))
        .bind(org)
        .bind(id)
        .opt(db)
        .await?
        .ok_or_else(|| AppError::not_found("Device"))
}

/// Adds a discovered device as a new item. Only what the computer reported is filled in; the
/// person can add or correct the rest. A serial is only recorded if one was reported.
pub async fn add_device(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("assets:write")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let kind = input.req("type", Str::req(1, 80));
    let name = input.str("name", Str::req(1, 160).optional()).into_option();
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let d = find_device(&mut tx, user.org_id, id).await?;
    if d["state"] == "linked" {
        return Err(AppError::new(409, "This device is already linked to an item"));
    }
    let category = crate::routes::catalog::resolve_category(&mut tx, user.org_id, &kind).await?.ok_or_else(|| {
        AppError::Invalid(vec![crate::validate::Issue::new(
            "type",
            format!("\"{kind}\" is not a category in this workspace"),
        )])
    })?;
    crate::routes::plans::check_limit(&mut tx, user.org_id, "items").await?;
    let n: i32 = Q::new(
        "UPDATE organizations SET next_asset_number = next_asset_number + 1 WHERE id = $1 RETURNING next_asset_number - 1",
    )
    .bind(user.org_id)
    .scalar(&mut tx)
    .await?;
    let name = name.unwrap_or_else(|| d["name"].as_str().unwrap_or("Device").to_string());
    let asset: Uuid = Q::new(
        "INSERT INTO assets (org_id, asset_tag, name, type, serial, manufacturer, model, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(user.org_id)
    .bind(format!("DT-{n}"))
    .bind(&name)
    .bind(&category)
    .bind(d["serial"].as_str())
    .bind(d["manufacturer"].as_str())
    .bind(d["model"].as_str())
    .bind(user.id)
    .scalar(&mut tx)
    .await?;
    let mut changes = Map::new();
    for (k, v) in [
        ("name", json!(name)),
        ("type", json!(category)),
        ("serial", d["serial"].clone()),
        ("manufacturer", d["manufacturer"].clone()),
        ("model", d["model"].clone()),
    ] {
        if !v.is_null() {
            changes.insert(k.into(), json!({ "from": null, "to": v }));
        }
    }
    changes.insert(
        "source".into(),
        json!({ "from": null, "to": format!("found by {}", d["agentName"].as_str().unwrap_or("device companion")) }),
    );
    record_event(&mut tx, user.org_id, asset, user.id, "created", Value::Object(changes)).await?;
    Q::new("UPDATE discovered_devices SET state = 'linked', asset_id = $2 WHERE id = $1")
        .bind(id)
        .bind(asset)
        .run(&mut tx)
        .await?;
    let item = find_asset(&mut tx, user.org_id, asset).await?;
    tx.commit().await?;
    Ok(Json(item))
}

pub async fn link_device(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("assets:write")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let asset = input.uuid("assetId", false, false).into_option();
    input.strict()?;
    let asset = asset.unwrap();
    let mut tx = state.pool.begin().await?;
    let d = find_device(&mut tx, user.org_id, id).await?;
    find_asset(&mut tx, user.org_id, asset).await?;
    Q::new("UPDATE discovered_devices SET state = 'linked', asset_id = $2 WHERE id = $1")
        .bind(id)
        .bind(asset)
        .run(&mut tx)
        .await?;
    let link = json!({ "deviceId": id, "name": d["name"], "computer": d["agentName"] });
    record_event(
        &mut tx,
        user.org_id,
        asset,
        user.id,
        "device_linked",
        json!({ "device": { "from": null, "to": link } }),
    )
    .await?;
    let row = find_device(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok(Json(row))
}

/// Ignore, or put back for review (which also unlinks).
pub async fn set_device_state(
    State(state): State<AppState>,
    user: User,
    Path((id, action)): Path<(String, String)>,
) -> Result<Json<Value>> {
    user.require("assets:write")?;
    let id = path_uuid(&id)?;
    let next = match action.as_str() {
        "ignore" => "ignored",
        "restore" => "new",
        _ => return Err(AppError::new(404, "Not found")),
    };
    let mut tx = state.pool.begin().await?;
    find_device(&mut tx, user.org_id, id).await?;
    Q::new("UPDATE discovered_devices SET state = $2, asset_id = NULL WHERE id = $1")
        .bind(id)
        .bind(next)
        .run(&mut tx)
        .await?;
    let row = find_device(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok(Json(row))
}

// ---- Agent side ----------------------------------------------------------------------------

struct Agent {
    id: Uuid,
    org_id: Uuid,
    paused: bool,
}

async fn agent_from(parts: &Parts, db: &mut Db) -> Result<Agent> {
    let token = parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .filter(|t| t.starts_with(TOKEN_PREFIX))
        .ok_or_else(|| {
            AppError::coded(401, "agent_unpaired", "This computer is not paired. Pair it again from the web app.")
        })?;
    let row = Q::new(
        "UPDATE device_agents SET last_seen_at = now() WHERE token_hash = $1 AND revoked_at IS NULL
          RETURNING id, org_id, paused",
    )
    .bind(tokens::hash(token))
    .opt(db)
    .await?
    .ok_or_else(|| AppError::coded(401, "agent_revoked", "This computer was disconnected from its workspace."))?;
    Ok(Agent {
        id: uuid_of(&row["id"]).unwrap(),
        org_id: uuid_of(&row["org_id"]).unwrap(),
        paused: row["paused"] == true,
    })
}

/// Exchanges a one-time pairing code for this computer's credential.
pub async fn pair(State(state): State<AppState>, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    let mut input = Input::body(&body);
    let code = input.req("code", Str::req(1, 40));
    let name = input.req("name", Str::req(1, 120));
    let platform = input.one_of("platform", &["windows", "linux", "macos"], false, false).into_option();
    let version = input.str("version", Str::text(40)).into_option();
    input.strict()?;
    let code: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase();
    let mut tx = state.pool.begin().await?;
    let paired = Q::new(
        "UPDATE agent_pairing_codes SET used_at = now()
          WHERE code_hash = $1 AND used_at IS NULL AND expires_at > now() RETURNING org_id, created_by",
    )
    .bind(tokens::hash(&code))
    .opt(&mut tx)
    .await?
    .ok_or_else(|| {
        AppError::coded(400, "code_invalid", "That pairing code is wrong, expired or already used. Create a new one.")
    })?;
    let org = uuid_of(&paired["org_id"]).unwrap();
    crate::routes::plans::check_limit(&mut tx, org, "devices").await?;
    let token = format!("{TOKEN_PREFIX}{}", tokens::new_token());
    let id: Uuid = Q::new(
        "INSERT INTO device_agents (org_id, name, platform, agent_version, token_hash, created_by, last_seen_at)
         VALUES ($1, $2, $3, $4, $5, $6, now()) RETURNING id",
    )
    .bind(org)
    .bind(name)
    .bind(platform)
    .bind(version)
    .bind(tokens::hash(&token))
    .bind(uuid_of(&paired["created_by"]))
    .scalar(&mut tx)
    .await?;
    let workspace: String = Q::new("SELECT name FROM organizations WHERE id = $1").bind(org).scalar(&mut tx).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "agentId": id, "token": token, "workspace": workspace }))))
}

pub async fn status(State(state): State<AppState>, parts: Parts) -> Result<Json<Value>> {
    let mut db = state.pool.acquire().await?;
    let agent = agent_from(&parts, &mut db).await?;
    let workspace: String =
        Q::new("SELECT name FROM organizations WHERE id = $1").bind(agent.org_id).scalar(&mut db).await?;
    Ok(Json(json!({ "paused": agent.paused, "workspace": workspace })))
}

/// The agent reports the devices it sees. With `complete: true` (a full scan), devices it saw
/// before but not now are marked missing.
pub async fn report(State(state): State<AppState>, parts: Parts, Body(body): Body) -> Result<Json<Value>> {
    let mut db = state.pool.acquire().await?;
    let agent = agent_from(&parts, &mut db).await?;
    drop(db);
    if agent.paused {
        return Ok(Json(json!({ "paused": true, "accepted": 0 })));
    }
    let mut input = Input::body(&body);
    let complete = input.boolean_or("complete", false);
    let version = input.str("version", Str::text(40)).into_option();
    let devices = input.raw_value("devices").and_then(|v| v.as_array().cloned()).unwrap_or_default();
    input.done()?;
    if devices.len() > 2000 {
        return Err(AppError::new(413, "Too many devices in one report"));
    }
    let text = |d: &Value, k: &str, max: usize| -> Option<String> {
        d[k].as_str().map(|s| s.trim().chars().take(max).collect::<String>()).filter(|s| !s.is_empty())
    };
    let mut tx = state.pool.begin().await?;
    let mut seen = HashSet::new();
    let mut moved = 0;
    for d in &devices {
        let Some(local_id) = text(d, "localId", 400) else { continue };
        let class = text(d, "class", 20).filter(|c| CLASSES.contains(&c.as_str())).unwrap_or_else(|| "other".into());
        let manufacturer = text(d, "manufacturer", 120);
        let serial = text(d, "serial", 120).and_then(|s| real_serial(&s));
        let fp = fingerprint(agent.id, &class, manufacturer.as_deref(), serial.as_deref(), &local_id);
        // The same device listed twice in one report counts once.
        if !seen.insert(fp.clone()) {
            continue;
        }
        let name = text(d, "name", 160).unwrap_or_else(|| "Unnamed device".into());
        let details = if d["details"].is_object() { d["details"].clone() } else { json!({}) };
        let previous: Option<Value> = Q::new(
            "INSERT INTO discovered_devices (org_id, agent_id, fingerprint, class, name, manufacturer, model, serial, local_id, details)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (org_id, fingerprint) DO UPDATE SET
               agent_id = EXCLUDED.agent_id, class = EXCLUDED.class, name = EXCLUDED.name,
               manufacturer = COALESCE(EXCLUDED.manufacturer, discovered_devices.manufacturer),
               model = COALESCE(EXCLUDED.model, discovered_devices.model),
               local_id = EXCLUDED.local_id, details = EXCLUDED.details, last_seen_at = now(), missing_since = NULL
             RETURNING (SELECT row_to_json(x) FROM (SELECT o.agent_id, o.asset_id FROM discovered_devices o
                         WHERE o.org_id = $1 AND o.fingerprint = $3) x) AS before, id, asset_id",
        )
        .bind(agent.org_id)
        .bind(agent.id)
        .bind(&fp)
        .bind(&class)
        .bind(&name)
        .bind(&manufacturer)
        .bind(text(d, "model", 120))
        .bind(&serial)
        .bind(&local_id)
        .bind(details)
        .opt(&mut tx)
        .await?;
        // A device now seen by a different computer has moved; the linked item's history says so.
        if let Some(row) = previous {
            let before = &row["before"];
            if let (Some(old), Some(asset)) = (uuid_of(&before["agent_id"]), uuid_of(&row["asset_id"])) {
                if old != agent.id {
                    moved += 1;
                    let names = Q::new("SELECT (SELECT name FROM device_agents WHERE id = $1) AS old, (SELECT name FROM device_agents WHERE id = $2) AS new")
                        .bind(old)
                        .bind(agent.id)
                        .one(&mut tx)
                        .await?;
                    let actor: Option<Uuid> = Q::new("SELECT created_by FROM device_agents WHERE id = $1")
                        .bind(agent.id)
                        .scalar(&mut tx)
                        .await?;
                    if let Some(actor) = actor {
                        record_event(
                            &mut tx,
                            agent.org_id,
                            asset,
                            actor,
                            "seen_on_other_computer",
                            json!({ "computer": { "from": names["old"], "to": names["new"] } }),
                        )
                        .await?;
                    }
                }
            }
        }
    }
    let mut missing = 0;
    if complete {
        missing = Q::new(
            "UPDATE discovered_devices SET missing_since = now()
              WHERE org_id = $1 AND agent_id = $2 AND missing_since IS NULL AND NOT (fingerprint = ANY($3))",
        )
        .bind(agent.org_id)
        .bind(agent.id)
        .bind(seen.iter().cloned().collect::<Vec<_>>())
        .run(&mut tx)
        .await?;
    }
    if version.is_some() {
        Q::new("UPDATE device_agents SET agent_version = $2 WHERE id = $1")
            .bind(agent.id)
            .bind(version)
            .run(&mut tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({ "paused": false, "accepted": seen.len(), "moved": moved, "missing": missing })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_serials_identify_nothing() {
        assert_eq!(real_serial("To Be Filled By O.E.M."), None);
        assert_eq!(real_serial("0000000"), None);
        assert_eq!(real_serial(" 5CD1234XYZ "), Some("5CD1234XYZ".into()));
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        // Same serial and maker: the same device on any computer.
        assert_eq!(
            fingerprint(a, "monitor", Some("Dell"), Some("cn0abc"), "x"),
            fingerprint(b, "monitor", Some("DELL "), Some("CN0ABC"), "y")
        );
        // No serial: never matched across computers, even with the same model.
        assert_ne!(
            fingerprint(a, "monitor", Some("Dell"), None, "USB\\1"),
            fingerprint(b, "monitor", Some("Dell"), None, "USB\\1")
        );
    }
}
