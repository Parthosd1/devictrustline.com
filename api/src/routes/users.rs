use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{self, User, ROLES};
use crate::db::{Db, Q};
use crate::error::{AppError, Result};
use crate::routes::auth::password;
use crate::validate::{path_uuid, Input, Str, EMAIL};
use crate::web::Body;
use crate::AppState;

const COLUMNS: &str = r#"u.id, u.email, u.name, m.role, (m.is_active AND u.is_active) AS "isActive",
    u.last_login_at AS "lastLoginAt", m.created_at AS "createdAt", m.access_scope AS "accessScope",
    COALESCE((SELECT json_agg(g.inventory_id ORDER BY g.created_at) FROM inventory_grants g
               WHERE g.org_id = m.org_id AND g.user_id = m.user_id), '[]'::json) AS "inventoryIds""#;

/// Which inventories a member reaches: everything, or the listed inventories and everything
/// inside them. Owners and admins always reach everything.
pub struct Access {
    pub scope: String,
    pub inventories: Vec<Uuid>,
}

impl Access {
    /// Reads `accessScope` and `inventoryIds`; None when neither is given.
    pub fn read(input: &mut Input) -> Option<Access> {
        let scope = input.one_of("accessScope", &["all", "inventories"], true, false);
        let ids = input.raw_value("inventoryIds");
        if scope.is_absent() && ids.is_none() {
            return None;
        }
        let mut inventories = vec![];
        match &ids {
            None | Some(Value::Null) => {}
            Some(Value::Array(items)) if items.len() <= 100 => {
                for item in items {
                    match item.as_str().and_then(crate::validate::parse_uuid) {
                        Some(id) if !inventories.contains(&id) => inventories.push(id),
                        Some(_) => {}
                        None => input.fail("inventoryIds", "Must be a list of inventory ids"),
                    }
                }
            }
            Some(_) => input.fail("inventoryIds", "Must be a list of at most 100 inventory ids"),
        }
        let scope =
            scope.into_option().unwrap_or_else(|| if inventories.is_empty() { "all" } else { "inventories" }.into());
        if scope == "inventories" && inventories.is_empty() {
            input.fail("inventoryIds", "Choose at least one inventory");
        }
        if scope == "all" {
            inventories.clear();
        }
        Some(Access { scope, inventories })
    }

    pub fn all() -> Access {
        Access { scope: "all".into(), inventories: vec![] }
    }

    /// Owners and admins manage the whole workspace, so their access cannot be limited.
    pub fn check_role(&self, role: &str) -> Result<()> {
        if self.scope != "all" && ["owner", "admin"].contains(&role) {
            return Err(AppError::new(400, "Owners and admins always have access to every inventory"));
        }
        Ok(())
    }

    /// Every listed inventory must be an active inventory of this workspace.
    pub async fn check_inventories(&self, db: &mut Db, org: Uuid) -> Result<()> {
        if self.inventories.is_empty() {
            return Ok(());
        }
        let found: i64 =
            Q::new("SELECT count(*) FROM inventories WHERE org_id = $1 AND id = ANY($2) AND archived_at IS NULL")
                .bind(org)
                .bind(self.inventories.clone())
                .scalar(&mut *db)
                .await?;
        if found != self.inventories.len() as i64 {
            return Err(AppError::new(400, "Inventory not found"));
        }
        Ok(())
    }

    /// Stores this access on a membership, replacing the grants it had.
    pub async fn apply(&self, db: &mut Db, org: Uuid, member: Uuid, by: Option<Uuid>) -> Result<()> {
        Q::new("UPDATE memberships SET access_scope = $3 WHERE org_id = $1 AND user_id = $2")
            .bind(org)
            .bind(member)
            .bind(&self.scope)
            .run(&mut *db)
            .await?;
        Q::new("DELETE FROM inventory_grants WHERE org_id = $1 AND user_id = $2 AND NOT (inventory_id = ANY($3))")
            .bind(org)
            .bind(member)
            .bind(self.inventories.clone())
            .run(&mut *db)
            .await?;
        Q::new(
            "INSERT INTO inventory_grants (org_id, user_id, inventory_id, created_by)
             SELECT $1, $2, unnest($3::uuid[]), $4 ON CONFLICT DO NOTHING",
        )
        .bind(org)
        .bind(member)
        .bind(self.inventories.clone())
        .bind(by)
        .run(&mut *db)
        .await?;
        Ok(())
    }
}

/// Only an owner hands out or takes away ownership.
pub fn check_owner_change(user: &User, role: &str) -> Result<()> {
    if role == "owner" && !auth::can(&user.role, "workspace:own") {
        return Err(AppError::new(403, "Only an owner can make someone an owner"));
    }
    Ok(())
}

async fn member(db: &mut crate::db::Db, org_id: uuid::Uuid, id: uuid::Uuid) -> Result<Value> {
    Q::new(format!(
        "SELECT {COLUMNS} FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.org_id = $1 AND m.user_id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .opt(db)
    .await?
    .ok_or_else(|| AppError::not_found("User"))
}

pub async fn list(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("users:read")?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!(
        "SELECT {COLUMNS} FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.org_id = $1 ORDER BY u.name"
    ))
    .bind(user.org_id)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

/// Creates an account with a temporary password the admin shares. People who already have an
/// account (in any workspace) are invited instead, so nobody can take over an existing login.
pub async fn create(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("users:manage")?;
    let mut input = Input::body(&body);
    let name = input.req("name", Str::req(1, 120));
    let email = input.req("email", EMAIL);
    let role = input.one_of("role", ROLES, false, false).into_option().unwrap_or_default();
    let access = Access::read(&mut input).unwrap_or_else(Access::all);
    let password = password(&mut input, "password", false);
    input.done()?;
    check_owner_change(&user, &role)?;
    access.check_role(&role)?;
    let hash = auth::hash_password(password.unwrap()).await?;
    let mut tx = state.pool.begin().await?;
    crate::routes::plans::check_limit(&mut tx, user.org_id, "members").await?;
    access.check_inventories(&mut tx, user.org_id).await?;
    let id: Option<uuid::Uuid> = Q::new(
        "INSERT INTO users (email, name, password_hash) VALUES ($1::citext, $2, $3) ON CONFLICT (email) DO NOTHING RETURNING id",
    )
    .bind(&email)
    .bind(&name)
    .bind(hash)
    .scalars(&mut tx)
    .await?
    .into_iter()
    .next();
    let Some(id) = id else {
        return Err(AppError::coded(
            409,
            "account_exists",
            "This email already has a DeviceTrustline account. Send them an invitation instead.",
        ));
    };
    Q::new("INSERT INTO memberships (org_id, user_id, role) VALUES ($1, $2, $3)")
        .bind(user.org_id)
        .bind(id)
        .bind(&role)
        .run(&mut tx)
        .await?;
    access.apply(&mut tx, user.org_id, id, Some(user.id)).await?;
    let created = member(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("users:manage")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let name = input.str("name", Str::req(1, 120).optional());
    let role = input.one_of("role", ROLES, true, false);
    let is_active = input.boolean("isActive", false);
    let access = Access::read(&mut input);
    let password = password(&mut input, "password", true);
    input.strict()?;
    let hash = match password {
        Some(p) => Some(auth::hash_password(p).await?),
        None => None,
    };

    let mut tx = state.pool.begin().await?;
    // Lock the workspace's owners so two concurrent edits cannot remove the last one.
    let owners =
        Q::new("SELECT user_id FROM memberships WHERE org_id = $1 AND role = 'owner' AND is_active FOR UPDATE")
            .bind(user.org_id)
            .all(&mut tx)
            .await?;
    let current = Q::new("SELECT role, is_active FROM memberships WHERE user_id = $1 AND org_id = $2")
        .bind(id)
        .bind(user.org_id)
        .opt(&mut tx)
        .await?
        .ok_or_else(|| AppError::not_found("User"))?;
    let current_role = current["role"].as_str().unwrap_or("").to_string();
    let current_owner = current_role == "owner" && current["is_active"] == true;
    let next_role = role.value().cloned().unwrap_or_else(|| current_role.clone());
    let next_active = is_active.value().copied().unwrap_or(current["is_active"] == true);
    // Admins manage everyone except owners; only an owner changes an owner or makes one.
    if current_role == "owner" || next_role == "owner" {
        check_owner_change(&user, "owner")?;
    }
    if current_owner && !(next_role == "owner" && next_active) && owners.len() <= 1 {
        return Err(AppError::new(409, "A workspace needs at least one active owner"));
    }
    // A new role of owner or admin reaches every inventory; other role changes keep their access.
    let access = match access {
        Some(a) => {
            a.check_role(&next_role)?;
            Some(a)
        }
        None if ["owner", "admin"].contains(&next_role.as_str()) => Some(Access::all()),
        None => None,
    };
    if let Some(a) = &access {
        a.check_inventories(&mut tx, user.org_id).await?;
    }
    if name.is_present() || hash.is_some() {
        // The account itself (name, password) belongs to its owner once they use other workspaces.
        let others: i64 = Q::new("SELECT count(*) FROM memberships WHERE user_id = $1 AND org_id <> $2")
            .bind(id)
            .bind(user.org_id)
            .scalar(&mut tx)
            .await?;
        if others > 0 && id != user.id {
            return Err(AppError::new(
                403,
                "This person also uses other workspaces, so only they can change their name or password",
            ));
        }
        Q::new(
            "UPDATE users SET
               name = COALESCE($2, name),
               password_hash = COALESCE($3, password_hash),
               session_version = session_version + CASE WHEN $3::text IS NULL THEN 0 ELSE 1 END,
               updated_at = now()
             WHERE id = $1",
        )
        .bind(id)
        .bind(name.into_option())
        .bind(hash)
        .run(&mut tx)
        .await?;
    }
    // Widen access before the role becomes owner or admin, narrow it after the role changes.
    if let Some(a) = access.as_ref().filter(|a| a.scope == "all") {
        a.apply(&mut tx, user.org_id, id, Some(user.id)).await?;
    }
    Q::new("UPDATE memberships SET role = COALESCE($3, role), is_active = COALESCE($4, is_active) WHERE user_id = $1 AND org_id = $2")
        .bind(id)
        .bind(user.org_id)
        .bind(role.into_option())
        .bind(is_active.into_option())
        .run(&mut tx)
        .await?;
    if let Some(a) = access.filter(|a| a.scope != "all") {
        a.apply(&mut tx, user.org_id, id, Some(user.id)).await?;
    }
    let updated = member(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok(Json(updated))
}
