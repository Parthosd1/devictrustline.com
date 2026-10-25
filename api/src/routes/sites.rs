use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::User;
use crate::db::Q;
use crate::error::{AppError, Result};
use crate::validate::{path_uuid, Input, Str};
use crate::web::Body;
use crate::AppState;

pub async fn list_buildings(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("sites:read")?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(
        r#"SELECT b.id, b.name, b.address,
                  (SELECT count(*)::int FROM assets a WHERE a.building_id = b.id AND a.status <> 'Retired') AS "assetCount",
                  COALESCE((SELECT json_agg(json_build_object(
                              'id', l.id, 'name', l.name,
                              'assetCount', (SELECT count(*)::int FROM assets a WHERE a.location_id = l.id AND a.status <> 'Retired'))
                            ORDER BY l.name)
                            FROM locations l WHERE l.building_id = b.id), '[]') AS locations
             FROM buildings b WHERE b.org_id = $1 ORDER BY b.name"#,
    )
    .bind(user.org_id)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn create_building(
    State(state): State<AppState>,
    user: User,
    Body(body): Body,
) -> Result<(StatusCode, Json<Value>)> {
    user.require("sites:write")?;
    let mut input = Input::body(&body);
    let name = input.req("name", Str::req(1, 120));
    let address = input.str("address", Str::req(0, 300).nullish());
    input.done()?;
    let mut db = state.pool.acquire().await?;
    let mut b = Q::new("INSERT INTO buildings (org_id, name, address) VALUES ($1, $2, $3) RETURNING id, name, address")
        .bind(user.org_id)
        .bind(name)
        .bind(address.into_option())
        .one(&mut db)
        .await?;
    b["assetCount"] = json!(0);
    b["locations"] = json!([]);
    Ok((StatusCode::CREATED, Json(b)))
}

pub async fn update_building(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("sites:write")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let name = input.str("name", Str::req(1, 120).optional());
    let address = input.str("address", Str::req(0, 300).nullish());
    input.strict()?;
    let mut db = state.pool.acquire().await?;
    let b = Q::new(
        "UPDATE buildings SET name = COALESCE($3, name),
                address = CASE WHEN $4::boolean THEN $5 ELSE address END
          WHERE id = $1 AND org_id = $2 RETURNING id, name, address",
    )
    .bind(id)
    .bind(user.org_id)
    .bind(name.into_option())
    .bind(address.is_present())
    .bind(address.into_option())
    .opt(&mut db)
    .await?
    .ok_or_else(|| AppError::not_found("Building"))?;
    Ok(Json(b))
}

/// Deleting a building or location that assets still reference is refused with a clear message.
async fn delete_or_explain(state: &AppState, table: &str, id: Uuid, org_id: Uuid, what: &str) -> Result<StatusCode> {
    let mut db = state.pool.acquire().await?;
    match Q::new(format!("DELETE FROM {table} WHERE id = $1 AND org_id = $2")).bind(id).bind(org_id).run(&mut db).await
    {
        Ok(0) => Err(AppError::not_found(what)),
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(e) if e.pg_code().as_deref() == Some("23503") => {
            Err(AppError::new(409, format!("{what} still has assets. Move them first.")))
        }
        Err(e) => Err(e),
    }
}

pub async fn delete_building(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<StatusCode> {
    user.require("sites:write")?;
    delete_or_explain(&state, "buildings", path_uuid(&id)?, user.org_id, "Building").await
}

fn location_name(body: &Value) -> Result<String> {
    let mut input = Input::body(body);
    let name = input.req("name", Str::req(1, 120));
    input.done()?;
    Ok(name)
}

pub async fn create_location(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<(StatusCode, Json<Value>)> {
    user.require("sites:write")?;
    let building_id = path_uuid(&id)?;
    let name = location_name(&body)?;
    let mut db = state.pool.acquire().await?;
    Q::new("SELECT id FROM buildings WHERE id = $1 AND org_id = $2")
        .bind(building_id)
        .bind(user.org_id)
        .opt(&mut db)
        .await?
        .ok_or_else(|| AppError::not_found("Building"))?;
    let mut l = Q::new(
        r#"INSERT INTO locations (org_id, building_id, name) VALUES ($1, $2, $3) RETURNING id, name, building_id AS "buildingId""#,
    )
    .bind(user.org_id)
    .bind(building_id)
    .bind(name)
    .one(&mut db)
    .await?;
    l["assetCount"] = json!(0);
    Ok((StatusCode::CREATED, Json(l)))
}

pub async fn update_location(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("sites:write")?;
    let name = location_name(&body)?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let l = Q::new(r#"UPDATE locations SET name = $3 WHERE id = $1 AND org_id = $2 RETURNING id, name, building_id AS "buildingId""#)
        .bind(id)
        .bind(user.org_id)
        .bind(name)
        .opt(&mut db)
        .await?
        .ok_or_else(|| AppError::not_found("Location"))?;
    Ok(Json(l))
}

pub async fn delete_location(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<StatusCode> {
    user.require("sites:write")?;
    delete_or_explain(&state, "locations", path_uuid(&id)?, user.org_id, "Location").await
}
