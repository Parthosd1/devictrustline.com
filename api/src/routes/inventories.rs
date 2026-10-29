//! Inventories: a workspace's hubs and their sub-inventories (available, loaner, repair,
//! deployment, returned, a technician's own stock, a remote site...). Workspaces create, rename,
//! nest, archive and restore their own. Items move between them only through transfers.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::User;
use crate::db::{Db, Q};
use crate::error::{AppError, Result};
use crate::validate::{path_uuid, Input, Opt, Str};
use crate::web::Body;
use crate::AppState;

pub const KINDS: &[&str] =
    &["hub", "general", "available", "loaner", "repair", "deployment", "returned", "technician", "site"];

/// Every inventory of workspace $1 with its full path ("Main IT Hub / Loaner Inventory") and depth.
pub const TREE: &str = "WITH RECURSIVE tree AS (
    SELECT id, name::text AS path, ARRAY[lower(name)] AS sort, 0 AS depth FROM inventories WHERE org_id = $1 AND parent_id IS NULL
    UNION ALL
    SELECT c.id, tree.path || ' / ' || c.name, tree.sort || lower(c.name), tree.depth + 1
      FROM inventories c JOIN tree ON c.parent_id = tree.id WHERE tree.depth < 20
  )";

fn select() -> String {
    format!(
        r#"{TREE}
  SELECT i.id, i.parent_id AS "parentId", i.name, i.kind, i.description, tree.path, tree.depth,
         i.building_id AS "buildingId", b.name AS "buildingName", i.location_id AS "locationId", l.name AS "locationName",
         i.custodian_id AS "custodianId", cu.name AS "custodianName", i.requires_approval AS "requiresApproval",
         i.archived_at AS "archivedAt", i.created_at AS "createdAt", i.updated_at AS "updatedAt",
         (SELECT count(*)::int FROM assets a WHERE a.inventory_id = i.id AND a.archived_at IS NULL) AS "assetCount",
         (SELECT count(*)::int FROM inventory_transfers t
           WHERE t.status = 'pending' AND (t.to_inventory_id = i.id OR t.from_inventory_id = i.id)) AS "pendingTransfers"
    FROM inventories i
    JOIN tree ON tree.id = i.id
    LEFT JOIN buildings b ON b.id = i.building_id
    LEFT JOIN locations l ON l.id = i.location_id
    LEFT JOIN users cu ON cu.id = i.custodian_id"#
    )
}

pub async fn find(db: &mut Db, org: Uuid, id: Uuid) -> Result<Value> {
    Q::new(format!("{} WHERE i.org_id = $1 AND i.id = $2", select()))
        .bind(org)
        .bind(id)
        .opt(db)
        .await?
        .ok_or_else(|| AppError::not_found("Inventory"))
}

/// An inventory and every inventory below it.
pub async fn subtree_ids(db: &mut Db, org: Uuid, id: Uuid) -> Result<Vec<Uuid>> {
    Q::new(
        "WITH RECURSIVE t AS (
           SELECT id, 0 AS depth FROM inventories WHERE org_id = $1 AND id = $2
           UNION ALL SELECT c.id, t.depth + 1 FROM inventories c JOIN t ON c.parent_id = t.id WHERE t.depth < 20
         ) SELECT id FROM t",
    )
    .bind(org)
    .bind(id)
    .scalars(db)
    .await
}

pub async fn list(
    State(state): State<AppState>,
    user: User,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    user.require("inventory:read")?;
    let mut input = Input::query(&q);
    let archived = input.one_of_or("archived", &["false", "true", "all"], "false");
    input.done()?;
    let filter = match archived.as_str() {
        "false" => "AND i.archived_at IS NULL",
        "true" => "AND i.archived_at IS NOT NULL",
        _ => "",
    };
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!(
        "{} WHERE i.org_id = $1 AND ($2::uuid[] IS NULL OR i.id = ANY($2)) {filter} ORDER BY tree.sort COLLATE \"C\"",
        select()
    ))
    .bind(user.org_id)
    .bind(user.scope_param())
    .all(&mut db)
    .await?;
    let unassigned: i64 =
        Q::new("SELECT count(*) FROM assets WHERE org_id = $1 AND inventory_id IS NULL AND archived_at IS NULL AND $2::uuid[] IS NULL")
            .bind(user.org_id)
            .bind(user.scope_param())
            .scalar(&mut db)
            .await?;
    Ok(Json(json!({ "items": items, "unassigned": unassigned })))
}

pub async fn get(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("inventory:read")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let mut inv = find(&mut db, user.org_id, id).await?;
    inv["children"] = json!(
        Q::new(format!(
            "{} WHERE i.org_id = $1 AND i.parent_id = $2 AND ($3::uuid[] IS NULL OR i.id = ANY($3)) ORDER BY i.name",
            select()
        ))
        .bind(user.org_id)
        .bind(id)
        .bind(user.scope_param())
        .all(&mut db)
        .await?
    );
    inv["byStatus"] = Q::new(
        "SELECT COALESCE(json_object_agg(status, n), '{}') FROM
           (SELECT status, count(*)::int AS n FROM assets WHERE org_id = $1 AND inventory_id = $2 AND archived_at IS NULL GROUP BY status) s",
    )
    .bind(user.org_id)
    .bind(id)
    .scalar(&mut db)
    .await?;
    Ok(Json(inv))
}

struct Fields {
    name: Opt<String>,
    kind: Opt<String>,
    parent: Opt<Uuid>,
    description: Opt<String>,
    building: Opt<Uuid>,
    location: Opt<Uuid>,
    custodian: Opt<Uuid>,
    approval: Opt<bool>,
}

fn fields(input: &mut Input, partial: bool) -> Fields {
    Fields {
        name: input.str("name", if partial { Str::req(1, 120).optional() } else { Str::req(1, 120) }),
        kind: input.one_of("kind", KINDS, true, false),
        parent: input.uuid("parentId", true, true),
        description: input.str("description", Str::text(500)),
        building: input.uuid("buildingId", true, true),
        location: input.uuid("locationId", true, true),
        custodian: input.uuid("custodianId", true, true),
        approval: input.boolean("requiresApproval", false),
    }
}

async fn check_references(db: &mut Db, org: Uuid, f: &Fields, id: Option<Uuid>) -> Result<()> {
    if let Opt::Val(parent) = f.parent {
        let row = Q::new("SELECT archived_at IS NULL AS active FROM inventories WHERE org_id = $1 AND id = $2")
            .bind(org)
            .bind(parent)
            .opt(&mut *db)
            .await?
            .ok_or_else(|| AppError::new(400, "Parent inventory not found"))?;
        if row["active"] != true {
            return Err(AppError::new(400, "The parent inventory is archived"));
        }
        if let Some(id) = id {
            if subtree_ids(db, org, id).await?.contains(&parent) {
                return Err(AppError::new(
                    400,
                    "An inventory cannot be placed inside itself or one of its sub-inventories",
                ));
            }
        }
    }
    if let Opt::Val(custodian) = f.custodian {
        let active: i64 = Q::new("SELECT count(*) FROM memberships WHERE org_id = $1 AND user_id = $2 AND is_active")
            .bind(org)
            .bind(custodian)
            .scalar(&mut *db)
            .await?;
        if active == 0 {
            return Err(AppError::new(400, "The custodian must be an active member of this workspace"));
        }
    }
    Ok(())
}

pub async fn create(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("inventory:manage")?;
    let mut input = Input::body(&body);
    let f = fields(&mut input, false);
    input.strict()?;
    // Someone with limited access adds sub-inventories only inside what they were given.
    if user.scope.is_some() && !user.sees_inventory(f.parent.value().copied()) {
        return Err(AppError::new(400, "Parent inventory not found"));
    }
    let mut tx = state.pool.begin().await?;
    check_references(&mut tx, user.org_id, &f, None).await?;
    crate::routes::plans::check_limit(&mut tx, user.org_id, "inventories").await?;
    let kind = f.kind.value().cloned().unwrap_or_else(|| "general".into());
    let id: Uuid = Q::new(
        "INSERT INTO inventories (org_id, parent_id, name, kind, description, building_id, location_id, custodian_id,
                                  requires_approval, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
    )
    .bind(user.org_id)
    .bind(f.parent.into_option())
    .bind(f.name.into_option())
    .bind(kind)
    .bind(f.description.into_option())
    .bind(f.building.into_option())
    .bind(f.location.into_option())
    .bind(f.custodian.into_option())
    .bind(f.approval.into_option().unwrap_or(false))
    .bind(user.id)
    .scalar(&mut tx)
    .await
    .map_err(name_taken)?;
    let created = find(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(created)))
}

fn name_taken(e: AppError) -> AppError {
    if e.constraint() == Some("inventories_name_uniq") {
        AppError::new(409, "There is already an inventory with that name here")
    } else {
        e
    }
}

pub async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("inventory:manage")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let f = fields(&mut input, true);
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let current = find(&mut tx, user.org_id, id).await?;
    if !current["archivedAt"].is_null() {
        return Err(AppError::new(409, "Restore this inventory before changing it"));
    }
    if user.scope.is_some() && f.parent.is_present() && !user.sees_inventory(f.parent.value().copied()) {
        return Err(AppError::new(400, "Parent inventory not found"));
    }
    check_references(&mut tx, user.org_id, &f, Some(id)).await?;
    // A building without a location clears the old location.
    let location = match (&f.building, &f.location) {
        (Opt::Absent, l) | (_, l @ Opt::Val(_)) => l.clone(),
        (_, _) => Opt::Null,
    };
    Q::new(
        "UPDATE inventories SET
           name = COALESCE($3, name), kind = COALESCE($4, kind),
           parent_id = CASE WHEN $5 THEN $6 ELSE parent_id END,
           description = CASE WHEN $7 THEN $8 ELSE description END,
           building_id = CASE WHEN $9 THEN $10 ELSE building_id END,
           location_id = CASE WHEN $11 THEN $12 ELSE location_id END,
           custodian_id = CASE WHEN $13 THEN $14 ELSE custodian_id END,
           requires_approval = COALESCE($15, requires_approval), updated_at = now()
         WHERE org_id = $1 AND id = $2",
    )
    .bind(user.org_id)
    .bind(id)
    .bind(f.name.into_option())
    .bind(f.kind.into_option())
    .bind(f.parent.is_present())
    .bind(f.parent.into_option())
    .bind(f.description.is_present())
    .bind(f.description.into_option())
    .bind(f.building.is_present())
    .bind(f.building.into_option())
    .bind(location.is_present())
    .bind(location.into_option())
    .bind(f.custodian.is_present())
    .bind(f.custodian.into_option())
    .bind(f.approval.into_option())
    .run(&mut tx)
    .await
    .map_err(name_taken)?;
    let updated = find(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok(Json(updated))
}

/// Archives an empty inventory. Items and open transfers must be moved or settled first, and
/// sub-inventories archived first, so nothing is left somewhere nobody can see.
pub async fn archive(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("inventory:manage")?;
    let id = path_uuid(&id)?;
    let mut tx = state.pool.begin().await?;
    Q::new("SELECT 1 FROM inventories WHERE org_id = $1 AND id = $2 FOR UPDATE")
        .bind(user.org_id)
        .bind(id)
        .run(&mut tx)
        .await?;
    let inv = find(&mut tx, user.org_id, id).await?;
    let children: i64 =
        Q::new("SELECT count(*) FROM inventories WHERE org_id = $1 AND parent_id = $2 AND archived_at IS NULL")
            .bind(user.org_id)
            .bind(id)
            .scalar(&mut tx)
            .await?;
    let items: i64 = Q::new("SELECT count(*) FROM assets WHERE org_id = $1 AND inventory_id = $2")
        .bind(user.org_id)
        .bind(id)
        .scalar(&mut tx)
        .await?;
    if children > 0 {
        return Err(AppError::new(409, "Archive or move its sub-inventories first"));
    }
    if items > 0 {
        return Err(AppError::new(409, format!("Transfer the {items} item(s) in this inventory somewhere else first")));
    }
    if inv["pendingTransfers"].as_i64().unwrap_or(0) > 0 {
        return Err(AppError::new(409, "Approve, reject or cancel its pending transfers first"));
    }
    Q::new("UPDATE inventories SET archived_at = COALESCE(archived_at, now()), updated_at = now() WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(id)
        .run(&mut tx)
        .await?;
    let out = find(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

pub async fn restore(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("inventory:manage")?;
    let id = path_uuid(&id)?;
    let mut tx = state.pool.begin().await?;
    let inv = find(&mut tx, user.org_id, id).await?;
    if let Some(parent) = crate::db::uuid_of(&inv["parentId"]) {
        if !find(&mut tx, user.org_id, parent).await?["archivedAt"].is_null() {
            return Err(AppError::new(409, "Restore its parent inventory first"));
        }
    }
    Q::new("UPDATE inventories SET archived_at = NULL, updated_at = now() WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(id)
        .run(&mut tx)
        .await
        .map_err(name_taken)?;
    let out = find(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok(Json(out))
}
