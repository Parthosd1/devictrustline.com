//! Warranties: any number per item (manufacturer, extended, other), each with its dates,
//! provider, what it covers, proof of purchase and a claim reference.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::auth::User;
use crate::db::{uuid_of, Assign, Db, Param, Q};
use crate::error::{AppError, Result};
use crate::routes::assets::{find_asset, record_event};
use crate::validate::{path_uuid, Input, Opt, Str, DATE};
use crate::web::Body;
use crate::AppState;

pub const KINDS: &[&str] = &["manufacturer", "extended", "other"];

/// The workspace's "today" (in its time zone) and how many days count as "expiring soon".
pub const TODAY: &str = "(now() AT TIME ZONE o.timezone)::date";
pub const SOON: &str = "COALESCE((o.settings->'reminders'->>'expiringSoonDays')::int, 30)";

/// SQL for the state of a warranty ending on `ends` (a column or expression).
pub fn status_sql(ends: &str) -> String {
    format!(
        "CASE WHEN {ends} IS NULL THEN 'unknown' WHEN {ends} < {TODAY} THEN 'expired'
              WHEN {ends} <= {TODAY} + {SOON} THEN 'expiring_soon' ELSE 'active' END"
    )
}

fn select() -> String {
    format!(
        r#"SELECT w.id, w.asset_id AS "assetId", w.kind, w.provider, w.starts_on AS "startsOn", w.ends_on AS "endsOn",
                  w.coverage, w.claim_reference AS "claimReference", w.proof_attachment_id AS "proofAttachmentId",
                  f.filename AS "proofFilename", {} AS status,
                  CASE WHEN w.ends_on IS NULL THEN NULL ELSE w.ends_on - {TODAY} END AS "daysLeft",
                  w.created_at AS "createdAt", w.updated_at AS "updatedAt"
             FROM warranties w JOIN organizations o ON o.id = w.org_id
             LEFT JOIN attachments f ON f.id = w.proof_attachment_id"#,
        status_sql("w.ends_on")
    )
}

/// API field, column, cast.
const COLUMNS: &[(&str, &str, &str)] = &[
    ("kind", "kind", ""),
    ("provider", "provider", ""),
    ("startsOn", "starts_on", "::date"),
    ("endsOn", "ends_on", "::date"),
    ("coverage", "coverage", ""),
    ("claimReference", "claim_reference", ""),
    ("proofAttachmentId", "proof_attachment_id", "::uuid"),
];

fn fields(input: &mut Input, partial: bool) -> Vec<(&'static str, Opt<String>)> {
    let proof = match input.uuid("proofAttachmentId", true, true) {
        Opt::Val(u) => Opt::Val(u.to_string()),
        Opt::Null => Opt::Null,
        Opt::Absent => Opt::Absent,
    };
    let mut kind = input.one_of("kind", KINDS, true, false);
    if !partial && kind.is_absent() {
        kind = Opt::Val("manufacturer".into());
    }
    vec![
        ("kind", kind),
        ("provider", input.str("provider", Str::text(160))),
        ("startsOn", input.str("startsOn", DATE)),
        ("endsOn", input.str("endsOn", DATE)),
        ("coverage", input.str("coverage", Str::text(2000))),
        ("claimReference", input.str("claimReference", Str::text(160))),
        ("proofAttachmentId", proof),
    ]
}

async fn check_proof(db: &mut Db, org: Uuid, asset: Uuid, f: &[(&str, Opt<String>)]) -> Result<()> {
    if let Some((_, Opt::Val(id))) = f.iter().find(|(k, _)| *k == "proofAttachmentId") {
        let ok: i64 = Q::new("SELECT count(*) FROM attachments WHERE org_id = $1 AND asset_id = $2 AND id = $3::uuid")
            .bind(org)
            .bind(asset)
            .bind(id)
            .scalar(db)
            .await?;
        if ok == 0 {
            return Err(AppError::new(400, "Proof must be a file attached to this item"));
        }
    }
    Ok(())
}

fn check_dates(f: &[(&str, Opt<String>)], current: Option<&Value>) -> Result<()> {
    let get = |k: &str| match f.iter().find(|(key, _)| *key == k).map(|(_, v)| v) {
        Some(Opt::Val(v)) => Some(v.clone()),
        Some(Opt::Null) => None,
        _ => current.and_then(|c| c[k].as_str().map(str::to_string)),
    };
    if let (Some(s), Some(e)) = (get("startsOn"), get("endsOn")) {
        if e < s {
            return Err(AppError::Invalid(vec![crate::validate::Issue::new(
                "endsOn",
                "Must be on or after the start date",
            )]));
        }
    }
    Ok(())
}

pub async fn list(State(state): State<AppState>, user: User, Path(asset): Path<String>) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let asset = path_uuid(&asset)?;
    let mut db = state.pool.acquire().await?;
    find_asset(&mut db, user.org_id, asset).await?;
    let items = Q::new(format!(
        "{} WHERE w.org_id = $1 AND w.asset_id = $2 ORDER BY w.ends_on NULLS LAST, w.created_at",
        select()
    ))
    .bind(user.org_id)
    .bind(asset)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn create(
    State(state): State<AppState>,
    user: User,
    Path(asset): Path<String>,
    Body(body): Body,
) -> Result<(StatusCode, Json<Value>)> {
    user.require("assets:write")?;
    let asset = path_uuid(&asset)?;
    let mut input = Input::body(&body);
    let f = fields(&mut input, false);
    input.strict()?;
    check_dates(&f, None)?;
    let mut tx = state.pool.begin().await?;
    find_asset(&mut tx, user.org_id, asset).await?;
    check_proof(&mut tx, user.org_id, asset, &f).await?;
    let present: Vec<(&str, Value)> = f.iter().filter_map(|(k, v)| v.to_json().map(|j| (*k, j))).collect();
    let mut cols = vec!["org_id".to_string(), "asset_id".into()];
    let mut vals = vec!["$1".to_string(), "$2".into()];
    for (i, (key, _)) in present.iter().enumerate() {
        let (_, column, cast) = COLUMNS.iter().find(|(k, _, _)| k == key).unwrap();
        cols.push(column.to_string());
        vals.push(format!("${}{cast}", i + 3));
    }
    let mut q =
        Q::new(format!("INSERT INTO warranties ({}) VALUES ({}) RETURNING id", cols.join(", "), vals.join(", ")))
            .bind(user.org_id)
            .bind(asset);
    for (_, v) in &present {
        q = q.bind(v.as_str().map(str::to_string));
    }
    let id: Uuid = q.scalar(&mut tx).await?;
    let changes: Map<String, Value> =
        present.iter().map(|(k, v)| (k.to_string(), json!({ "from": null, "to": v }))).collect();
    let mut changes = changes;
    changes.insert("warrantyId".into(), json!({ "from": null, "to": id }));
    record_event(&mut tx, user.org_id, asset, user.id, "warranty_added", Value::Object(changes)).await?;
    let row = Q::new(format!("{} WHERE w.id = $1", select())).bind(id).one(&mut tx).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("assets:write")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let f = fields(&mut input, true);
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let current = Q::new(format!("{} WHERE w.org_id = $1 AND w.id = $2", select()))
        .bind(user.org_id)
        .bind(id)
        .opt(&mut tx)
        .await?
        .ok_or_else(|| AppError::not_found("Warranty"))?;
    let asset = uuid_of(&current["assetId"]).unwrap();
    check_dates(&f, Some(&current))?;
    check_proof(&mut tx, user.org_id, asset, &f).await?;
    let mut assign = Assign::new(3);
    let mut changes = Map::new();
    for (key, value) in &f {
        if let Some(to) = value.to_json() {
            if to != current[*key] {
                let (_, column, cast) = COLUMNS.iter().find(|(k, _, _)| k == key).unwrap();
                assign.set(column, cast, Param::Text(to.as_str().map(str::to_string)));
                changes.insert(key.to_string(), json!({ "from": current[*key], "to": to }));
            }
        }
    }
    if !assign.is_empty() {
        let sql = format!("UPDATE warranties SET {}, updated_at = now() WHERE org_id = $1 AND id = $2", assign.sql());
        assign.bind_to(Q::new(sql).bind(user.org_id).bind(id)).run(&mut tx).await?;
        changes.insert("warrantyId".into(), json!({ "from": id, "to": id }));
        record_event(&mut tx, user.org_id, asset, user.id, "warranty_updated", Value::Object(changes)).await?;
    }
    let row = Q::new(format!("{} WHERE w.id = $1", select())).bind(id).one(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(row))
}

pub async fn delete(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<StatusCode> {
    user.require("assets:write")?;
    let id = path_uuid(&id)?;
    let mut tx = state.pool.begin().await?;
    let row = Q::new("DELETE FROM warranties WHERE org_id = $1 AND id = $2 RETURNING asset_id, kind, ends_on")
        .bind(user.org_id)
        .bind(id)
        .opt(&mut tx)
        .await?
        .ok_or_else(|| AppError::not_found("Warranty"))?;
    let asset = uuid_of(&row["asset_id"]).unwrap();
    let removed = json!({ "id": id, "kind": row["kind"], "endsOn": row["ends_on"] });
    record_event(
        &mut tx,
        user.org_id,
        asset,
        user.id,
        "warranty_removed",
        json!({ "warranty": { "from": removed, "to": null } }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The single "warranty expires" date on an item (the older API, the item form and CSV import)
/// edits the item's first manufacturer warranty, creating or removing it as needed.
pub async fn set_primary(db: &mut Db, org: Uuid, asset: Uuid, ends: Option<&str>) -> Result<()> {
    let primary: Option<Uuid> = Q::new(
        "SELECT id FROM warranties WHERE org_id = $1 AND asset_id = $2 AND kind = 'manufacturer' ORDER BY created_at LIMIT 1",
    )
    .bind(org)
    .bind(asset)
    .scalars(&mut *db)
    .await?
    .into_iter()
    .next();
    match (primary, ends) {
        (Some(id), Some(date)) => {
            Q::new("UPDATE warranties SET ends_on = $2::date, updated_at = now() WHERE id = $1")
                .bind(id)
                .bind(date)
                .run(db)
                .await?
        }
        (Some(id), None) => Q::new("DELETE FROM warranties WHERE id = $1").bind(id).run(db).await?,
        (None, Some(date)) => {
            Q::new("INSERT INTO warranties (org_id, asset_id, kind, ends_on) VALUES ($1, $2, 'manufacturer', $3::date)")
                .bind(org)
                .bind(asset)
                .bind(date)
                .run(db)
                .await?
        }
        (None, None) => 0,
    };
    Ok(())
}
