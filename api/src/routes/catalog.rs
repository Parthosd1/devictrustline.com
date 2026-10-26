//! What a workspace describes its items with: categories, tags and custom fields. Nothing here is
//! fixed; new workspaces start with a few categories they can rename or remove.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::User;
use crate::db::{Db, Q};
use crate::error::{AppError, Result};
use crate::validate::{path_uuid, Input, Str};
use crate::web::Body;
use crate::AppState;

const BUSINESS_CATEGORIES: &[&str] = &[
    "Laptop",
    "Desktop",
    "Workstation",
    "Monitor",
    "Scanner",
    "Printer",
    "Docking Station",
    "Phone",
    "Tablet",
    "Network",
    "Accessory",
    "Other",
];
const PERSONAL_CATEGORIES: &[&str] =
    &["Electronics", "Appliances", "Furniture", "Tools", "Sports & Outdoors", "Vehicles", "Jewelry & Watches", "Other"];
pub const FIELD_TYPES: &[&str] = &["text", "number", "date", "boolean", "select"];

pub async fn seed_workspace(db: &mut Db, org: Uuid, kind: &str) -> Result<()> {
    let names = if kind == "personal" { PERSONAL_CATEGORIES } else { BUSINESS_CATEGORIES };
    Q::new("INSERT INTO categories (org_id, name) SELECT $1, unnest($2::text[])")
        .bind(org)
        .bind(names.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        .run(db)
        .await?;
    Ok(())
}

/// The workspace's category with this name (any letter case), as it is spelled there.
pub async fn resolve_category(db: &mut Db, org: Uuid, name: &str) -> Result<Option<String>> {
    Ok(Q::new("SELECT name FROM categories WHERE org_id = $1 AND lower(name) = lower($2)")
        .bind(org)
        .bind(name.trim())
        .scalars(db)
        .await?
        .into_iter()
        .next())
}

pub async fn category_names(db: &mut Db, org: Uuid) -> Result<Vec<String>> {
    Q::new("SELECT name FROM categories WHERE org_id = $1 ORDER BY lower(name)").bind(org).scalars(db).await
}

pub async fn list_categories(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(
        r#"SELECT c.id, c.name, (SELECT count(*)::int FROM assets a WHERE a.category_id = c.id) AS "itemCount"
             FROM categories c WHERE c.org_id = $1 ORDER BY lower(c.name)"#,
    )
    .bind(user.org_id)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn create_category(
    State(state): State<AppState>,
    user: User,
    Body(body): Body,
) -> Result<(StatusCode, Json<Value>)> {
    user.require("catalog:manage")?;
    let mut input = Input::body(&body);
    let name = input.req("name", Str::req(1, 80));
    input.strict()?;
    let mut db = state.pool.acquire().await?;
    let row = Q::new(r#"INSERT INTO categories (org_id, name) VALUES ($1, $2) RETURNING id, name, 0 AS "itemCount""#)
        .bind(user.org_id)
        .bind(name)
        .one(&mut db)
        .await?;
    Ok((StatusCode::CREATED, Json(row)))
}

/// Renaming a category renames it on every item in it.
pub async fn update_category(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("catalog:manage")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let name = input.req("name", Str::req(1, 80));
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let n = Q::new("UPDATE categories SET name = $3 WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(id)
        .bind(&name)
        .run(&mut tx)
        .await?;
    if n == 0 {
        return Err(AppError::not_found("Category"));
    }
    Q::new("UPDATE assets SET type = $3, updated_at = now() WHERE org_id = $1 AND category_id = $2")
        .bind(user.org_id)
        .bind(id)
        .bind(&name)
        .run(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "name": name })))
}

pub async fn delete_category(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<StatusCode> {
    user.require("catalog:manage")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let used: i64 = Q::new("SELECT count(*) FROM assets WHERE org_id = $1 AND category_id = $2")
        .bind(user.org_id)
        .bind(id)
        .scalar(&mut db)
        .await?;
    if used > 0 {
        return Err(AppError::new(
            409,
            format!("{used} items use this category. Move them to another category first."),
        ));
    }
    let n =
        Q::new("DELETE FROM categories WHERE org_id = $1 AND id = $2").bind(user.org_id).bind(id).run(&mut db).await?;
    if n == 0 {
        return Err(AppError::not_found("Category"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_tags(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(
        r#"SELECT t.id, t.name, (SELECT count(*)::int FROM asset_tags x WHERE x.tag_id = t.id) AS "itemCount"
             FROM tags t WHERE t.org_id = $1 ORDER BY lower(t.name)"#,
    )
    .bind(user.org_id)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn update_tag(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("catalog:manage")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let name = input.req("name", Str::req(1, 40));
    input.strict()?;
    let mut db = state.pool.acquire().await?;
    Q::new("UPDATE tags SET name = $3 WHERE org_id = $1 AND id = $2 RETURNING id, name")
        .bind(user.org_id)
        .bind(id)
        .bind(name)
        .opt(&mut db)
        .await?
        .map(Json)
        .ok_or_else(|| AppError::not_found("Tag"))
}

pub async fn delete_tag(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<StatusCode> {
    user.require("catalog:manage")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let n = Q::new("DELETE FROM tags WHERE org_id = $1 AND id = $2").bind(user.org_id).bind(id).run(&mut db).await?;
    if n == 0 {
        return Err(AppError::not_found("Tag"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Sets an item's tags to exactly these names, creating tags the workspace does not have yet.
pub async fn set_tags(db: &mut Db, org: Uuid, asset: Uuid, names: &[String]) -> Result<()> {
    Q::new("INSERT INTO tags (org_id, name) SELECT $1, unnest($2::text[]) ON CONFLICT DO NOTHING")
        .bind(org)
        .bind(names.to_vec())
        .run(&mut *db)
        .await?;
    let lower: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    Q::new(
        "DELETE FROM asset_tags x USING tags t
          WHERE x.asset_id = $2 AND t.id = x.tag_id AND x.org_id = $1 AND NOT (lower(t.name) = ANY($3))",
    )
    .bind(org)
    .bind(asset)
    .bind(lower.clone())
    .run(&mut *db)
    .await?;
    Q::new(
        "INSERT INTO asset_tags (org_id, asset_id, tag_id)
         SELECT $1, $2, id FROM tags WHERE org_id = $1 AND lower(name) = ANY($3) ON CONFLICT DO NOTHING",
    )
    .bind(org)
    .bind(asset)
    .bind(lower)
    .run(db)
    .await?;
    Ok(())
}

const FIELD_SELECT: &str = r#"SELECT f.id, f.label, f.field_type AS "type", f.options, f.category_id AS "categoryId",
       c.name AS "categoryName", f.position
  FROM custom_fields f LEFT JOIN categories c ON c.id = f.category_id"#;

pub async fn list_fields(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!("{FIELD_SELECT} WHERE f.org_id = $1 ORDER BY f.position, lower(f.label)"))
        .bind(user.org_id)
        .all(&mut db)
        .await?;
    Ok(Json(json!({ "items": items })))
}

fn field_options(input: &mut Input, kind: Option<&str>) -> Option<Vec<String>> {
    let raw = input.raw_value("options")?;
    let options: Option<Vec<String>> = raw.as_array().and_then(|a| {
        a.iter().map(|v| v.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && s.len() <= 80)).collect()
    });
    match options {
        Some(o) if !o.is_empty() && o.len() <= 50 => Some(o),
        _ if kind != Some("select") && raw.as_array().is_some_and(|a| a.is_empty()) => Some(vec![]),
        _ => {
            input.fail("options", "List 1 to 50 choices, each up to 80 characters");
            None
        }
    }
}

pub async fn create_field(
    State(state): State<AppState>,
    user: User,
    Body(body): Body,
) -> Result<(StatusCode, Json<Value>)> {
    user.require("catalog:manage")?;
    let mut input = Input::body(&body);
    let label = input.req("label", Str::req(1, 80));
    let kind = input.one_of("type", FIELD_TYPES, false, false).into_option();
    let options = field_options(&mut input, kind.as_deref());
    let category = input.uuid("categoryId", true, true).into_option();
    let position = input.int("position", 0, 10_000, 0);
    if kind.as_deref() == Some("select") && options.is_none() {
        input.fail("options", "Choices are required for a select field");
    }
    input.strict()?;
    let mut db = state.pool.acquire().await?;
    let id: Uuid = Q::new(
        "INSERT INTO custom_fields (org_id, label, field_type, options, category_id, position)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(user.org_id)
    .bind(label)
    .bind(&kind)
    .bind(json!(if kind.as_deref() == Some("select") { options.unwrap_or_default() } else { vec![] }))
    .bind(category)
    .bind(position as i32)
    .scalar(&mut db)
    .await?;
    let row = Q::new(format!("{FIELD_SELECT} WHERE f.id = $1")).bind(id).one(&mut db).await?;
    Ok((StatusCode::CREATED, Json(row)))
}

/// A field's type cannot change once created (stored values would no longer fit); its label,
/// choices, category and position can.
pub async fn update_field(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("catalog:manage")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let label = input.str("label", Str::req(1, 80).optional());
    let options = field_options(&mut input, Some("select"));
    let category = input.uuid("categoryId", true, true);
    let position = input.raw_value("position").and_then(|v| v.as_i64());
    input.strict()?;
    let mut db = state.pool.acquire().await?;
    let n = Q::new(
        "UPDATE custom_fields SET
           label = COALESCE($3, label),
           options = CASE WHEN field_type = 'select' AND $4::jsonb IS NOT NULL THEN $4::jsonb ELSE options END,
           category_id = CASE WHEN $5 THEN $6 ELSE category_id END,
           position = COALESCE($7, position)
         WHERE org_id = $1 AND id = $2",
    )
    .bind(user.org_id)
    .bind(id)
    .bind(label.into_option())
    .bind(options.map(|o| json!(o)))
    .bind(category.is_present() || matches!(category, crate::validate::Opt::Null))
    .bind(category.into_option())
    .bind(position.map(|p| p as i32))
    .run(&mut db)
    .await?;
    if n == 0 {
        return Err(AppError::not_found("Field"));
    }
    Ok(Json(Q::new(format!("{FIELD_SELECT} WHERE f.id = $1")).bind(id).one(&mut db).await?))
}

/// Removing a field also removes its values from every item.
pub async fn delete_field(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<StatusCode> {
    user.require("catalog:manage")?;
    let id = path_uuid(&id)?;
    let mut tx = state.pool.begin().await?;
    let n = Q::new("DELETE FROM custom_fields WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(id)
        .run(&mut tx)
        .await?;
    if n == 0 {
        return Err(AppError::not_found("Field"));
    }
    Q::new("UPDATE assets SET custom = custom - $2 WHERE org_id = $1 AND custom ? $2")
        .bind(user.org_id)
        .bind(id.to_string())
        .run(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Checks custom field values against the workspace's field definitions. Null removes a value.
pub async fn check_custom(db: &mut Db, org: Uuid, values: &serde_json::Map<String, Value>) -> Result<Value> {
    let fields = Q::new("SELECT id::text AS id, label, field_type, options FROM custom_fields WHERE org_id = $1")
        .bind(org)
        .all(db)
        .await?;
    let mut issues = vec![];
    for (key, value) in values {
        let path = format!("custom.{key}");
        let Some(f) = fields.iter().find(|f| f["id"] == key.as_str()) else {
            issues.push(crate::validate::Issue::new(&path, "Unknown field"));
            continue;
        };
        if value.is_null() {
            continue;
        }
        let ok = match f["field_type"].as_str().unwrap_or("") {
            "text" => value.as_str().is_some_and(|s| s.chars().count() <= 2000),
            "number" => value.is_number(),
            "date" => value.as_str().is_some_and(crate::validate::is_date),
            "boolean" => value.is_boolean(),
            "select" => f["options"].as_array().is_some_and(|o| o.contains(value)),
            _ => false,
        };
        if !ok {
            issues.push(crate::validate::Issue::new(
                &path,
                format!("Not a valid value for {}", f["label"].as_str().unwrap_or("")),
            ));
        }
    }
    if !issues.is_empty() {
        return Err(AppError::Invalid(issues));
    }
    Ok(Value::Object(values.clone()))
}
