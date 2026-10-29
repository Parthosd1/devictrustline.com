use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::auth::User;
use crate::db::{uuid_of, Assign, Db, Param, Q};
use crate::error::{AppError, Result};
use crate::validate::{path_uuid, Input, Opt, Str, DATE};
use crate::web::Body;
use crate::AppState;

pub const ASSET_STATUSES: &[&str] = &["Available", "Deployed", "Maintenance", "Retired"];
pub const CONDITIONS: &[&str] = &["New", "Good", "Fair", "Poor", "Broken"];

fn currency_check(s: &str) -> Option<&'static str> {
    (s.len() != 3 || !s.chars().all(|c| c.is_ascii_alphabetic())).then_some("Use a 3-letter currency code, like USD")
}

/// API field, column, and the cast that types a text parameter for that column. Every editable field.
const COLUMNS: &[(&str, &str, &str)] = &[
    ("assetTag", "asset_tag", ""),
    ("name", "name", ""),
    ("type", "type", ""),
    ("serial", "serial", ""),
    ("manufacturer", "manufacturer", ""),
    ("model", "model", ""),
    ("status", "status", ""),
    ("buildingId", "building_id", "::uuid"),
    ("locationId", "location_id", "::uuid"),
    ("parentId", "parent_asset_id", "::uuid"),
    ("notes", "notes", ""),
    ("purchaseDate", "purchase_date", "::date"),
    ("warrantyExpires", "warranty_expires", "::date"),
    ("condition", "condition", ""),
    ("ownerId", "owner_id", "::uuid"),
    ("purchasePrice", "purchase_price", "::numeric"),
    ("currency", "currency", ""),
    ("seller", "seller", ""),
    ("returnDeadline", "return_deadline", "::date"),
    ("maintenanceDue", "maintenance_due", "::date"),
];

pub const SELECT_TEMPLATE: &str = r#"
  SELECT a.id, a.asset_tag AS "assetTag", a.name, a.type, a.serial, a.manufacturer, a.model, a.status,
         a.building_id AS "buildingId", b.name AS "buildingName",
         a.location_id AS "locationId", l.name AS "locationName",
         a.parent_asset_id AS "parentId", pa.asset_tag AS "parentTag", pa.name AS "parentName",
         a.assigned_person_id AS "assignedPersonId", pe.name AS "assignedPersonName", a.assigned_at AS "assignedAt",
         (SELECT count(*)::int FROM assets c WHERE c.parent_asset_id = a.id) AS "childCount",
         a.notes, a.purchase_date AS "purchaseDate", a.warranty_expires AS "warrantyExpires",
         a.category_id AS "categoryId", a.condition, a.owner_id AS "ownerId", ow.name AS "ownerName",
         a.purchase_price AS "purchasePrice", a.currency, a.seller, a.return_deadline AS "returnDeadline",
         a.maintenance_due AS "maintenanceDue", a.custom, a.loan_due AS "loanDue",
         WARRANTY_STATUS AS "warrantyStatus",
         COALESCE((SELECT json_agg(t.name ORDER BY lower(t.name)) FROM asset_tags x JOIN tags t ON t.id = x.tag_id
                    WHERE x.asset_id = a.id), '[]') AS tags,
         (SELECT count(*)::int FROM attachments f WHERE f.asset_id = a.id) AS "attachmentCount",
         (SELECT f.id FROM attachments f WHERE f.asset_id = a.id AND f.kind = 'photo' ORDER BY f.created_at LIMIT 1) AS "photoId",
         a.inventory_id AS "inventoryId", inv.name AS "inventoryName", inv.custodian_id AS "custodianId",
         (SELECT name FROM users WHERE id = inv.custodian_id) AS "custodianName",
         a.archived_at AS "archivedAt", a.created_at AS "createdAt", a.updated_at AS "updatedAt"
    FROM assets a
    JOIN organizations o ON o.id = a.org_id
    LEFT JOIN users ow ON ow.id = a.owner_id
    LEFT JOIN buildings b ON b.id = a.building_id
    LEFT JOIN locations l ON l.id = a.location_id
    LEFT JOIN assets pa ON pa.id = a.parent_asset_id
    LEFT JOIN people pe ON pe.id = a.assigned_person_id
    LEFT JOIN inventories inv ON inv.id = a.inventory_id"#;

/// The item columns every asset endpoint returns.
pub static SELECT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    SELECT_TEMPLATE.replace("WARRANTY_STATUS", &crate::routes::warranties::status_sql("a.warranty_expires"))
});

/// The editable fields a request sent, in column order. Values are text or null.
struct Fields(Vec<(&'static str, Opt<String>)>);

impl Fields {
    fn get(&self, key: &str) -> &Opt<String> {
        &self.0.iter().find(|(k, _)| *k == key).unwrap().1
    }
    fn set(&mut self, key: &str, value: Opt<String>) {
        self.0.iter_mut().find(|(k, _)| *k == key).unwrap().1 = value;
    }
    fn present(&self) -> impl Iterator<Item = (&'static str, Value)> + '_ {
        self.0.iter().filter_map(|(k, v)| v.to_json().map(|j| (*k, j)))
    }
}

fn uuid_text(o: Opt<Uuid>) -> Opt<String> {
    match o {
        Opt::Val(u) => Opt::Val(u.to_string()),
        Opt::Null => Opt::Null,
        Opt::Absent => Opt::Absent,
    }
}

fn money_text(o: Opt<f64>) -> Opt<String> {
    match o {
        Opt::Val(n) => Opt::Val(format!("{n:.2}")),
        Opt::Null => Opt::Null,
        Opt::Absent => Opt::Absent,
    }
}

/// Fields set alongside the columns: tags (replaced as a set) and custom field values (merged).
struct Extras {
    tags: Option<Vec<String>>,
    custom: Option<serde_json::Map<String, Value>>,
}

fn extras(input: &mut Input) -> Extras {
    let tags = input.raw_value("tags").and_then(|v| {
        let list: Option<Vec<String>> = v.as_array().and_then(|a| {
            a.iter()
                .map(|t| t.as_str().map(|t| t.trim().to_string()).filter(|t| (1..=40).contains(&t.chars().count())))
                .collect()
        });
        match list {
            Some(mut l) if l.len() <= 20 => {
                l.sort_by_key(|t| t.to_lowercase());
                l.dedup_by_key(|t| t.to_lowercase());
                Some(l)
            }
            _ => {
                input.fail("tags", "Up to 20 tags, each 1 to 40 characters");
                None
            }
        }
    });
    let custom = input.raw_value("custom").and_then(|v| match v {
        Value::Object(m) => Some(m),
        _ => {
            input.fail("custom", "Expected object");
            None
        }
    });
    Extras { tags, custom }
}

/// Finishes validation together with the category check, so every problem is reported at once.
/// The category a request names must exist in the workspace; its stored spelling is used.
async fn finish_validation(input: Input, db: &mut Db, org: Uuid, fields: &mut Fields) -> Result<()> {
    let mut issues = match input.strict() {
        Ok(()) => vec![],
        Err(AppError::Invalid(issues)) => issues,
        Err(e) => return Err(e),
    };
    if let Opt::Val(name) = fields.get("type").clone() {
        match crate::routes::catalog::resolve_category(db, org, &name).await? {
            Some(canonical) => fields.set("type", Opt::Val(canonical)),
            None => issues.push(crate::validate::Issue::new(
                "type",
                format!("\"{name}\" is not a category in this workspace. Add it under Categories first."),
            )),
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(AppError::Invalid(issues))
    }
}

fn asset_fields(input: &mut Input, partial: bool) -> Fields {
    let name = if partial { Str::req(1, 160).optional() } else { Str::req(1, 160) };
    let kind = if partial { Str::req(1, 80).optional() } else { Str::req(1, 80) };
    let mut f = vec![
        ("assetTag", input.str("assetTag", Str::req(1, 40).optional())),
        ("name", input.str("name", name)),
        ("type", input.str("type", kind)),
        ("serial", input.str("serial", Str::text(120))),
        ("manufacturer", input.str("manufacturer", Str::text(120))),
        ("model", input.str("model", Str::text(120))),
        ("status", input.one_of("status", ASSET_STATUSES, true, false)),
        ("buildingId", uuid_text(input.uuid("buildingId", true, true))),
        ("locationId", uuid_text(input.uuid("locationId", true, true))),
        ("parentId", uuid_text(input.uuid("parentId", true, true))),
        ("notes", input.str("notes", Str::text(2000))),
        ("purchaseDate", input.str("purchaseDate", DATE)),
        ("warrantyExpires", input.str("warrantyExpires", DATE)),
        ("condition", input.one_of("condition", CONDITIONS, true, true)),
        ("ownerId", uuid_text(input.uuid("ownerId", true, true))),
        ("purchasePrice", money_text(input.money("purchasePrice"))),
        ("currency", input.str("currency", Str::text(3).check(currency_check))),
        ("seller", input.str("seller", Str::text(160))),
        ("returnDeadline", input.str("returnDeadline", DATE)),
        ("maintenanceDue", input.str("maintenanceDue", DATE)),
    ];
    if let Opt::Val(c) = &mut f.iter_mut().find(|(k, _)| *k == "currency").unwrap().1 {
        *c = c.to_uppercase();
    }
    let mut fields = Fields(f);
    if !partial && fields.get("status").is_absent() {
        fields.set("status", Opt::Val("Available".into()));
    }
    fields
}

pub async fn find_asset(db: &mut Db, org_id: Uuid, id: Uuid) -> Result<Value> {
    Q::new(format!("{} WHERE a.org_id = $1 AND a.id = $2", *SELECT))
        .bind(org_id)
        .bind(id)
        .opt(db)
        .await?
        .ok_or_else(|| AppError::not_found("Asset"))
}

pub async fn record_event(
    db: &mut Db,
    org_id: Uuid,
    asset_id: Uuid,
    actor_id: Uuid,
    kind: &str,
    changes: Value,
) -> Result<()> {
    Q::new("INSERT INTO asset_events (org_id, asset_id, actor_id, event_type, changes) VALUES ($1, $2, $3, $4, $5)")
        .bind(org_id)
        .bind(asset_id)
        .bind(actor_id)
        .bind(kind)
        .bind(changes)
        .run(db)
        .await?;
    Ok(())
}

/// Every asset below this one in the parent-child tree, nearest first. Rows are locked for update.
pub async fn descendant_ids(db: &mut Db, org_id: Uuid, id: Uuid) -> Result<Vec<Uuid>> {
    Q::new(
        "WITH RECURSIVE tree AS (
           SELECT id, 1 AS depth FROM assets WHERE org_id = $1 AND parent_asset_id = $2
           UNION ALL
           SELECT a.id, t.depth + 1 FROM assets a JOIN tree t ON a.parent_asset_id = t.id WHERE t.depth < 20
         )
         SELECT a.id FROM tree t JOIN assets a ON a.id = t.id ORDER BY t.depth FOR UPDATE OF a",
    )
    .bind(org_id)
    .bind(id)
    .scalars(db)
    .await
}

/// Where a parent asset sits: (building, location) as JSON values.
async fn parent_placement(db: &mut Db, org_id: Uuid, parent_id: Uuid) -> Result<(Value, Value)> {
    let parent = Q::new(
        r#"SELECT building_id AS "buildingId", location_id AS "locationId" FROM assets WHERE org_id = $1 AND id = $2"#,
    )
    .bind(org_id)
    .bind(parent_id)
    .opt(db)
    .await?
    .ok_or_else(|| AppError::new(400, "Parent asset not found"))?;
    Ok((parent["buildingId"].clone(), parent["locationId"].clone()))
}

fn json_to_opt(v: &Value) -> Opt<String> {
    v.as_str().map(|s| Opt::Val(s.to_string())).unwrap_or(Opt::Null)
}

pub async fn list(
    State(state): State<AppState>,
    user: User,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let mut db = state.pool.acquire().await?;
    let (items, total, limit, offset) = search(&mut db, &user, &query, 500).await?;
    Ok(Json(json!({ "items": items, "total": total, "limit": limit, "offset": offset })))
}

/// The items matching a list query: (items, total matching, limit, offset).
async fn search(
    db: &mut Db,
    user: &User,
    query: &HashMap<String, String>,
    max_limit: i64,
) -> Result<(Vec<Value>, i32, i64, i64)> {
    let mut input = Input::query(query);
    let q = input.str("q", Str::req(0, 200).optional());
    let status = input.one_of("status", ASSET_STATUSES, true, false);
    let kind = input.str("type", Str::req(1, 80).optional());
    let tag = input.str("tag", Str::req(1, 40).optional());
    let building = input.uuid("buildingId", true, false);
    let location = input.uuid("locationId", true, false);
    let person = input.uuid("personId", true, false);
    let parent = input.uuid("parentId", true, false);
    let category = input.uuid("categoryId", true, false);
    let owner = input.uuid("ownerId", true, false);
    // An inventory id, or "none" for items in no inventory; with includeSub=true, its sub-inventories too.
    let inventory = input.str("inventoryId", Str { trim: false, ..Str::req(1, 36).optional() }).into_option();
    let include_sub = input.one_of_or("includeSub", &["true", "false"], "false") == "true";
    let archived = input.one_of_or("archived", &["false", "true", "all"], "false");
    let limit = input.int("limit", 1, max_limit, 100.min(max_limit));
    let offset = input.int("offset", 0, i64::MAX, 0);
    input.done()?;

    let mut wheres = vec!["a.org_id = $1".to_string(), user.scope_sql("a", 2)];
    match archived.as_str() {
        "false" => wheres.push("a.archived_at IS NULL".into()),
        "true" => wheres.push("a.archived_at IS NOT NULL".into()),
        _ => {}
    }
    let mut text_params: Vec<String> = vec![];
    let mut uuid_params: Vec<Uuid> = vec![];
    // The scope is $2; text parameters come next ($3, $4...), then UUIDs, so each keeps its own type.
    let mut text = |sql: &str, v: String, wheres: &mut Vec<String>| {
        text_params.push(v);
        wheres.push(sql.replace('?', &format!("${}", text_params.len() + 2)));
    };
    if let Some(q) = q.into_option().filter(|q| !q.is_empty()) {
        let escaped: String =
            q.chars().flat_map(|c| if "\\%_".contains(c) { vec!['\\', c] } else { vec![c] }).collect();
        text(
            "(a.asset_tag ILIKE ? OR a.name ILIKE ? OR a.serial ILIKE ? OR a.type ILIKE ?
              OR a.model ILIKE ? OR a.manufacturer ILIKE ? OR b.name ILIKE ? OR l.name ILIKE ? OR pe.name ILIKE ?)",
            format!("%{escaped}%"),
            &mut wheres,
        );
    }
    if let Opt::Val(s) = status {
        text("a.status = ?", s, &mut wheres);
    }
    if let Opt::Val(t) = kind {
        text("lower(a.type) = lower(?)", t, &mut wheres);
    }
    if let Opt::Val(t) = tag {
        text(
            "EXISTS (SELECT 1 FROM asset_tags x JOIN tags t ON t.id = x.tag_id WHERE x.asset_id = a.id AND lower(t.name) = lower(?))",
            t,
            &mut wheres,
        );
    }
    let first_uuid = text_params.len() + 3;
    for (column, value) in [
        ("a.building_id", building),
        ("a.location_id", location),
        ("a.assigned_person_id", person),
        ("a.parent_asset_id", parent),
        ("a.category_id", category),
        ("a.owner_id", owner),
    ] {
        if let Opt::Val(u) = value {
            wheres.push(format!("{column} = ${}", first_uuid + uuid_params.len()));
            uuid_params.push(u);
        }
    }
    match inventory.as_deref() {
        None => {}
        Some("none") => wheres.push("a.inventory_id IS NULL".into()),
        Some(id) => {
            let Some(id) = crate::validate::parse_uuid(id) else {
                return Err(AppError::Invalid(vec![crate::validate::Issue::new("inventoryId", "Invalid uuid")]));
            };
            let n = first_uuid + uuid_params.len();
            wheres.push(if include_sub {
                format!(
                    "a.inventory_id IN (WITH RECURSIVE t AS (SELECT id, 0 AS d FROM inventories WHERE org_id = $1 AND id = ${n}
                       UNION ALL SELECT c.id, t.d + 1 FROM inventories c JOIN t ON c.parent_id = t.id WHERE t.d < 20) SELECT id FROM t)"
                )
            } else {
                format!("a.inventory_id = ${n}")
            });
            uuid_params.push(id);
        }
    }
    let where_sql = wheres.join(" AND ");
    let build = |sql: String| {
        let mut q = Q::new(sql).bind(user.org_id).bind(user.scope_param());
        for t in &text_params {
            q = q.bind(t.clone());
        }
        for u in &uuid_params {
            q = q.bind(*u);
        }
        q
    };
    let items = build(format!(
        "{} WHERE {where_sql} ORDER BY a.created_at DESC, a.asset_tag DESC LIMIT {limit} OFFSET {offset}",
        *SELECT
    ))
    .all(&mut *db)
    .await?;
    let total: i32 = build(format!(
        "SELECT count(*)::int AS total FROM assets a
           LEFT JOIN buildings b ON b.id = a.building_id
           LEFT JOIN locations l ON l.id = a.location_id
           LEFT JOIN people pe ON pe.id = a.assigned_person_id WHERE {where_sql}"
    ))
    .scalar(&mut *db)
    .await?;
    Ok((items, total, limit, offset))
}

/// The same items as the list, as a CSV file (up to 10,000 rows).
pub async fn export(
    State(state): State<AppState>,
    user: User,
    Query(mut query): Query<HashMap<String, String>>,
) -> Result<axum::response::Response> {
    use axum::response::IntoResponse;
    user.require("assets:read")?;
    query.entry("limit".into()).or_insert_with(|| "10000".into());
    let mut db = state.pool.acquire().await?;
    let (items, ..) = search(&mut db, &user, &query, 10_000).await?;
    let columns: &[(&str, &str)] = &[
        ("assetTag", "Asset tag"),
        ("name", "Name"),
        ("type", "Category"),
        ("manufacturer", "Brand"),
        ("model", "Model"),
        ("serial", "Serial"),
        ("status", "Status"),
        ("condition", "Condition"),
        ("buildingName", "Place"),
        ("locationName", "Location"),
        ("ownerName", "Owner"),
        ("assignedPersonName", "Assigned to"),
        ("purchaseDate", "Purchase date"),
        ("purchasePrice", "Price"),
        ("currency", "Currency"),
        ("seller", "Retailer"),
        ("warrantyExpires", "Warranty ends"),
        ("returnDeadline", "Return by"),
        ("maintenanceDue", "Maintenance due"),
        ("tags", "Tags"),
        ("notes", "Notes"),
        ("archivedAt", "Archived"),
    ];
    let cell = |v: &Value| -> String {
        let text = match v {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            Value::Array(a) => a.iter().filter_map(|t| t.as_str()).collect::<Vec<_>>().join("; "),
            other => other.to_string(),
        };
        // Quote every cell, and keep spreadsheet programs from running formulas in text.
        let text = if text.starts_with(['=', '+', '-', '@']) { format!("'{text}") } else { text };
        format!("\"{}\"", text.replace('"', "\"\""))
    };
    let mut csv = columns.iter().map(|(_, h)| cell(&json!(h))).collect::<Vec<_>>().join(",");
    csv.push_str("\r\n");
    for item in &items {
        csv.push_str(&columns.iter().map(|(k, _)| cell(&item[*k])).collect::<Vec<_>>().join(","));
        csv.push_str("\r\n");
    }
    let disposition = "attachment; filename=\"devicetrustline-items.csv\"".to_string();
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (axum::http::header::CONTENT_DISPOSITION, disposition),
        ],
        csv,
    )
        .into_response())
}

/// Archives an item: it leaves lists and reminders but keeps its history, and can be restored.
pub async fn archive(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    set_archived(state, user, id, true).await
}

pub async fn restore(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    set_archived(state, user, id, false).await
}

async fn set_archived(state: AppState, user: User, id: String, archive: bool) -> Result<Json<Value>> {
    user.require("assets:write")?;
    let id = path_uuid(&id)?;
    let mut tx = state.pool.begin().await?;
    let current = Q::new("SELECT archived_at FROM assets WHERE org_id = $1 AND id = $2 FOR UPDATE OF assets")
        .bind(user.org_id)
        .bind(id)
        .opt(&mut tx)
        .await?
        .ok_or_else(|| AppError::not_found("Asset"))?;
    if current["archived_at"].is_null() == archive {
        let at: Value = Q::new(
            "UPDATE assets SET archived_at = CASE WHEN $3 THEN now() END, updated_at = now()
              WHERE org_id = $1 AND id = $2 RETURNING COALESCE(to_json(archived_at), 'null'::json)",
        )
        .bind(user.org_id)
        .bind(id)
        .bind(archive)
        .scalar(&mut tx)
        .await?;
        let changes = json!({ "archivedAt": { "from": current["archived_at"], "to": at } });
        record_event(&mut tx, user.org_id, id, user.id, if archive { "archived" } else { "restored" }, changes).await?;
    }
    let asset = find_asset(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok(Json(asset))
}

/// Resolves a scanned code: an asset tag, a serial number, or a DeviceTrustline label URL (…?asset=DT-1001).
pub fn parse_scanned_code(raw: &str) -> String {
    let code = raw.trim();
    if let Ok(url) = url::Url::parse(code) {
        if let Some((_, tag)) = url.query_pairs().find(|(k, _)| k == "asset") {
            if !tag.is_empty() {
                return tag.trim().to_string();
            }
        }
    }
    code.to_string()
}

/// Finds the asset a scanned code refers to within one organization, preferring an asset tag match.
pub async fn find_by_code(db: &mut Db, org_id: Uuid, raw: &str) -> Result<Value> {
    let code = parse_scanned_code(raw);
    let mut asset = Q::new(format!(
        "{} WHERE a.org_id = $1 AND (lower(a.asset_tag) = lower($2) OR lower(a.serial) = lower($2))
          ORDER BY (lower(a.asset_tag) = lower($2)) DESC LIMIT 1",
        *SELECT
    ))
    .bind(org_id)
    .bind(&code)
    .opt(db)
    .await?
    .ok_or_else(|| AppError::new(404, format!("No asset matches “{code}”")))?;
    let by_tag = asset["assetTag"].as_str().unwrap_or("").to_lowercase() == code.to_lowercase();
    asset["matchedBy"] = json!(if by_tag { "assetTag" } else { "serial" });
    Ok(asset)
}

pub async fn lookup(
    State(state): State<AppState>,
    user: User,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let mut input = Input::query(&query);
    let code = input.req("code", Str::req(1, 500));
    input.done()?;
    let mut db = state.pool.acquire().await?;
    let asset = find_by_code(&mut db, user.org_id, &code).await?;
    user.require_asset(&asset)
        .map_err(|_| AppError::new(404, format!("No asset matches “{}”", parse_scanned_code(&code))))?;
    Ok(Json(asset))
}

pub async fn get(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let mut asset = find_asset(&mut db, user.org_id, id).await?;
    let children = Q::new(format!(
        "{} WHERE a.org_id = $1 AND a.parent_asset_id = $2 AND {} ORDER BY a.asset_tag",
        *SELECT,
        user.scope_sql("a", 3)
    ))
    .bind(user.org_id)
    .bind(id)
    .bind(user.scope_param())
    .all(&mut db)
    .await?;
    asset["children"] = json!(children);
    Ok(Json(asset))
}

pub async fn events(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    find_asset(&mut db, user.org_id, id).await?;
    let items = Q::new(
        r#"SELECT e.id, e.event_type AS "type", e.changes, e.created_at AS "createdAt",
                  CASE WHEN u.id IS NULL THEN NULL ELSE json_build_object('id', u.id, 'name', u.name) END AS actor
             FROM asset_events e LEFT JOIN users u ON u.id = e.actor_id
            WHERE e.org_id = $1 AND e.asset_id = $2 ORDER BY e.id DESC"#,
    )
    .bind(user.org_id)
    .bind(id)
    .all(&mut db)
    .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn create(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("assets:write")?;
    let mut input = Input::body(&body);
    let mut fields = asset_fields(&mut input, false);
    let extra = extras(&mut input);
    // Where a new item starts. Later moves are transfers, so updates do not take this field.
    let inventory = input.uuid("inventoryId", true, true).into_option();
    let org = user.org_id;
    let mut tx = state.pool.begin().await?;
    finish_validation(input, &mut tx, org, &mut fields).await?;
    crate::routes::plans::check_limit(&mut tx, org, "items").await?;
    if user.scope.is_some() && inventory.is_none() {
        return Err(AppError::Invalid(vec![crate::validate::Issue::new(
            "inventoryId",
            "Choose one of your inventories for this item",
        )]));
    }
    if let Some(inv) = inventory {
        if !user.sees_inventory(Some(inv)) {
            return Err(AppError::new(400, "Inventory not found"));
        }
        let active: i64 =
            Q::new("SELECT count(*) FROM inventories WHERE org_id = $1 AND id = $2 AND archived_at IS NULL")
                .bind(org)
                .bind(inv)
                .scalar(&mut tx)
                .await?;
        if active == 0 {
            return Err(AppError::new(400, "Inventory not found"));
        }
    }
    // The warranty date is stored as the item's manufacturer warranty.
    let warranty = fields.get("warrantyExpires").clone();
    fields.set("warrantyExpires", Opt::Absent);
    // A component placed inside a parent sits where the parent is unless told otherwise.
    if let Opt::Val(parent) = fields.get("parentId").clone() {
        if fields.get("buildingId").is_absent() && fields.get("locationId").is_absent() {
            let (b, l) = parent_placement(&mut tx, org, Uuid::parse_str(&parent).unwrap()).await?;
            fields.set("buildingId", json_to_opt(&b));
            fields.set("locationId", json_to_opt(&l));
        }
    }
    let n: i32 = Q::new(
        "UPDATE organizations SET next_asset_number = next_asset_number + 1 WHERE id = $1 RETURNING next_asset_number - 1",
    )
    .bind(org)
    .scalar(&mut tx)
    .await?;
    let tag = match fields.get("assetTag").clone() {
        Opt::Val(t) => t,
        _ => format!("DT-{n}"),
    };
    fields.set("assetTag", Opt::Absent);
    let present: Vec<(&str, Value)> = fields.present().collect();
    let mut cols = vec!["org_id".to_string(), "asset_tag".into(), "created_by".into()];
    let mut placeholders = vec!["$1".to_string(), "$2".into(), "$3".into()];
    let mut q_params = vec![];
    for (i, (key, value)) in present.iter().enumerate() {
        let (_, column, cast) = COLUMNS.iter().find(|(k, _, _)| k == key).unwrap();
        cols.push(column.to_string());
        placeholders.push(format!("${}{cast}", i + 4));
        q_params.push(value.as_str().map(str::to_string));
    }
    let mut q =
        Q::new(format!("INSERT INTO assets ({}) VALUES ({}) RETURNING id", cols.join(", "), placeholders.join(", ")))
            .bind(org)
            .bind(&tag)
            .bind(user.id);
    for p in q_params {
        q = q.bind(p);
    }
    let id: Uuid = q.scalar(&mut tx).await?;
    let mut changes: Map<String, Value> =
        present.iter().map(|(k, v)| (k.to_string(), json!({ "from": null, "to": v }))).collect();
    if let Some(custom) = &extra.custom {
        let custom = crate::routes::catalog::check_custom(&mut tx, org, custom).await?;
        Q::new("UPDATE assets SET custom = jsonb_strip_nulls($3) WHERE org_id = $1 AND id = $2")
            .bind(org)
            .bind(id)
            .bind(&custom)
            .run(&mut tx)
            .await?;
        changes.insert("custom".into(), json!({ "from": null, "to": custom }));
    }
    if let Some(tags) = extra.tags.filter(|t| !t.is_empty()) {
        crate::routes::catalog::set_tags(&mut tx, org, id, &tags).await?;
        changes.insert("tags".into(), json!({ "from": null, "to": tags }));
    }
    if let Opt::Val(date) = &warranty {
        crate::routes::warranties::set_primary(&mut tx, org, id, Some(date)).await?;
        changes.insert("warrantyExpires".into(), json!({ "from": null, "to": date }));
    }
    if let Some(inv) = inventory {
        Q::new("UPDATE assets SET inventory_id = $3 WHERE org_id = $1 AND id = $2")
            .bind(org)
            .bind(id)
            .bind(inv)
            .run(&mut tx)
            .await?;
        changes.insert("inventoryId".into(), json!({ "from": null, "to": inv }));
    }
    let created = find_asset(&mut tx, org, id).await?;
    record_event(&mut tx, org, id, user.id, "created", Value::Object(changes)).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(created)))
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
    let mut fields = asset_fields(&mut input, true);
    let extra = extras(&mut input);
    let org = user.org_id;
    let mut tx = state.pool.begin().await?;
    finish_validation(input, &mut tx, org, &mut fields).await?;
    let current = Q::new(
        r#"SELECT asset_tag AS "assetTag", name, type, serial, manufacturer, model, status, building_id AS "buildingId",
                  location_id AS "locationId", parent_asset_id AS "parentId", notes,
                  purchase_date AS "purchaseDate", warranty_expires AS "warrantyExpires",
                  condition, owner_id AS "ownerId", purchase_price::text AS "purchasePrice", currency, seller,
                  return_deadline AS "returnDeadline", maintenance_due AS "maintenanceDue", custom,
                  COALESCE((SELECT json_agg(t.name ORDER BY lower(t.name)) FROM asset_tags x JOIN tags t ON t.id = x.tag_id
                             WHERE x.asset_id = assets.id), '[]') AS tags
             FROM assets WHERE org_id = $1 AND id = $2 FOR UPDATE"#,
    )
    .bind(org)
    .bind(id)
    .opt(&mut tx)
    .await?
    .ok_or_else(|| AppError::not_found("Asset"))?;
    let descendants = descendant_ids(&mut tx, org, id).await?;

    if let Opt::Val(parent) = fields.get("parentId").clone() {
        if current["parentId"].as_str() != Some(parent.as_str()) {
            let parent_id = Uuid::parse_str(&parent).unwrap();
            if parent_id == id || descendants.contains(&parent_id) {
                return Err(AppError::new(400, "An asset cannot be placed inside itself or one of its own components"));
            }
            // Attaching to a new parent moves the asset to the parent's building and location.
            if fields.get("buildingId").is_absent() && fields.get("locationId").is_absent() {
                let (b, l) = parent_placement(&mut tx, org, parent_id).await?;
                fields.set("buildingId", json_to_opt(&b));
                fields.set("locationId", json_to_opt(&l));
            }
        }
    }
    // Moving to another building without naming a location clears the old location.
    if let Some(building) = fields.get("buildingId").to_json() {
        if building != current["buildingId"] && fields.get("locationId").is_absent() {
            fields.set("locationId", Opt::Null);
        }
    }
    let mut changes = Map::new();
    if let Some(to) = fields.get("warrantyExpires").to_json() {
        if to != current["warrantyExpires"] {
            crate::routes::warranties::set_primary(&mut tx, org, id, to.as_str()).await?;
            changes.insert("warrantyExpires".into(), json!({ "from": current["warrantyExpires"], "to": to }));
        }
        fields.set("warrantyExpires", Opt::Absent);
    }
    let mut assign = Assign::new(3);
    for (key, to) in fields.present() {
        if to != current[key] {
            let (_, column, cast) = COLUMNS.iter().find(|(k, _, _)| *k == key).unwrap();
            assign.set(column, cast, Param::Text(to.as_str().map(str::to_string)));
            changes.insert(key.to_string(), json!({ "from": current[key], "to": to }));
        }
    }
    let moved = changes.contains_key("buildingId") || changes.contains_key("locationId");
    if let Some(custom) = &extra.custom {
        let patch = crate::routes::catalog::check_custom(&mut tx, org, custom).await?;
        let next: Value = Q::new(
            "UPDATE assets SET custom = jsonb_strip_nulls(custom || $3), updated_at = now()
              WHERE org_id = $1 AND id = $2 RETURNING custom",
        )
        .bind(org)
        .bind(id)
        .bind(&patch)
        .scalar(&mut tx)
        .await?;
        if next != current["custom"] {
            changes.insert("custom".into(), json!({ "from": current["custom"], "to": next }));
        }
    }
    if let Some(tags) = &extra.tags {
        let before: Vec<String> = serde_json::from_value(current["tags"].clone()).unwrap_or_default();
        let lower = |v: &[String]| v.iter().map(|t| t.to_lowercase()).collect::<Vec<_>>();
        if lower(&before) != lower(tags) {
            crate::routes::catalog::set_tags(&mut tx, org, id, tags).await?;
            changes.insert("tags".into(), json!({ "from": before, "to": tags }));
        }
    }
    let column_changes = changes.keys().any(|k| k != "custom" && k != "tags" && k != "warrantyExpires");
    if column_changes {
        let sql = format!("UPDATE assets SET {}, updated_at = now() WHERE org_id = $1 AND id = $2", assign.sql());
        assign.bind_to(Q::new(sql).bind(org).bind(id)).run(&mut tx).await?;
    }
    if !changes.is_empty() {
        let kind = if changes.contains_key("status") { "status_changed" } else { "updated" };
        record_event(&mut tx, org, id, user.id, kind, Value::Object(changes)).await?;
    }
    // Components travel with their parent.
    if moved && !descendants.is_empty() {
        let (b, l) = parent_placement(&mut tx, org, id).await?;
        move_assets(&mut tx, org, user.id, &descendants, uuid_of(&b), uuid_of(&l), id).await?;
    }
    let asset = find_asset(&mut tx, org, id).await?;
    tx.commit().await?;
    Ok(Json(asset))
}

async fn move_assets(
    db: &mut Db,
    org_id: Uuid,
    actor_id: Uuid,
    ids: &[Uuid],
    building: Option<Uuid>,
    location: Option<Uuid>,
    via: Uuid,
) -> Result<()> {
    let rows = Q::new(
        r#"SELECT id, building_id AS "buildingId", location_id AS "locationId" FROM assets
            WHERE org_id = $1 AND id = ANY($2) AND (building_id IS DISTINCT FROM $3 OR location_id IS DISTINCT FROM $4)"#,
    )
    .bind(org_id)
    .bind(ids.to_vec())
    .bind(building)
    .bind(location)
    .all(db)
    .await?;
    for row in rows {
        let asset_id = uuid_of(&row["id"]).unwrap();
        Q::new(
            "UPDATE assets SET building_id = $3, location_id = $4, updated_at = now() WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(asset_id)
        .bind(building)
        .bind(location)
        .run(db)
        .await?;
        let mut changes = json!({ "via": { "from": null, "to": via } });
        if uuid_of(&row["buildingId"]) != building {
            changes["buildingId"] = json!({ "from": row["buildingId"], "to": building });
        }
        if uuid_of(&row["locationId"]) != location {
            changes["locationId"] = json!({ "from": row["locationId"], "to": location });
        }
        record_event(db, org_id, asset_id, actor_id, "moved_with_parent", changes).await?;
    }
    Ok(())
}

struct Person {
    id: Uuid,
    name: String,
}

async fn find_person(db: &mut Db, org_id: Uuid, id: Uuid) -> Result<Person> {
    let p = Q::new("SELECT id, name, is_active FROM people WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .opt(db)
        .await?
        .ok_or_else(|| AppError::new(400, "Person not found"))?;
    let name = p["name"].as_str().unwrap_or("").to_string();
    if p["is_active"] != true {
        return Err(AppError::new(409, format!("{name} is inactive")));
    }
    Ok(Person { id, name })
}

/// Assigns or returns an asset (and by default its components), recording one event per asset.
async fn set_assignment(
    db: &mut Db,
    user: &User,
    id: Uuid,
    person: Option<&Person>,
    status: &str,
    include_components: bool,
    note: Option<String>,
) -> Result<()> {
    let org = user.org_id;
    let root = Q::new("SELECT id, status FROM assets WHERE org_id = $1 AND id = $2 FOR UPDATE")
        .bind(org)
        .bind(id)
        .opt(&mut *db)
        .await?
        .ok_or_else(|| AppError::not_found("Asset"))?;
    if root["status"] == "Retired" {
        return Err(AppError::new(409, "Retired assets cannot be assigned or returned"));
    }
    let mut ids = vec![id];
    if include_components {
        ids.extend(descendant_ids(db, org, id).await?);
    }
    let rows = Q::new(
        r#"SELECT a.id, a.status, a.assigned_person_id AS "personId", p.name AS "personName"
             FROM assets a LEFT JOIN people p ON p.id = a.assigned_person_id
            WHERE a.org_id = $1 AND a.id = ANY($2) AND a.status <> 'Retired'"#,
    )
    .bind(org)
    .bind(ids)
    .all(&mut *db)
    .await?;
    for row in rows {
        let row_status = row["status"].as_str().unwrap_or("");
        let next_status = match person {
            Some(_) if row_status == "Available" => "Deployed",
            Some(_) => row_status,
            None => status,
        };
        let row_person = uuid_of(&row["personId"]);
        if row_person == person.map(|p| p.id) && row_status == next_status {
            continue;
        }
        let asset_id = uuid_of(&row["id"]).unwrap();
        Q::new(
            "UPDATE assets SET assigned_person_id = $3, assigned_at = CASE WHEN $3::uuid IS NULL THEN NULL ELSE now() END,
                    status = $4, updated_at = now() WHERE org_id = $1 AND id = $2",
        )
        .bind(org)
        .bind(asset_id)
        .bind(person.map(|p| p.id))
        .bind(next_status)
        .run(&mut *db)
        .await?;
        let from = row_person.map(|pid| json!({ "id": pid, "name": row["personName"] }));
        let to = person.map(|p| json!({ "id": p.id, "name": p.name }));
        let mut changes = json!({ "assignedTo": { "from": from, "to": to } });
        if next_status != row_status {
            changes["status"] = json!({ "from": row_status, "to": next_status });
        }
        if asset_id != id {
            changes["via"] = json!({ "from": null, "to": id });
        }
        if let Some(note) = &note {
            changes["note"] = json!({ "from": null, "to": note });
        }
        let kind = if person.is_some() { "assigned" } else { "returned" };
        record_event(db, org, asset_id, user.id, kind, changes).await?;
    }
    Ok(())
}

pub async fn assign(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("assets:write")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let person_id = input.uuid("personId", false, false);
    let include = input.boolean_or("includeComponents", true);
    let note = input.str("note", Str::text(500)).into_option();
    // When a loaned item is due back, if it is a loan.
    let due = input.str("dueDate", DATE).into_option();
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let person = find_person(&mut tx, user.org_id, person_id.into_option().unwrap()).await?;
    set_assignment(&mut tx, &user, id, Some(&person), "", include, note).await?;
    Q::new("UPDATE assets SET loan_due = $3::date WHERE org_id = $1 AND id = $2")
        .bind(user.org_id)
        .bind(id)
        .bind(due)
        .run(&mut tx)
        .await?;
    let asset = find_asset(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok(Json(asset))
}

pub async fn return_asset(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("assets:write")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let status = input.one_of_or("status", &["Available", "Maintenance"], "Available");
    let include = input.boolean_or("includeComponents", true);
    let note = input.str("note", Str::text(500)).into_option();
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    set_assignment(&mut tx, &user, id, None, &status, include, note).await?;
    Q::new(
        "UPDATE assets SET loan_due = NULL WHERE org_id = $1 AND assigned_person_id IS NULL AND loan_due IS NOT NULL",
    )
    .bind(user.org_id)
    .run(&mut tx)
    .await?;
    let asset = find_asset(&mut tx, user.org_id, id).await?;
    tx.commit().await?;
    Ok(Json(asset))
}

#[cfg(test)]
mod tests {
    use super::parse_scanned_code;

    #[test]
    fn scanned_codes() {
        assert_eq!(parse_scanned_code(" DT-7 "), "DT-7");
        assert_eq!(parse_scanned_code("https://x.test/path?asset=DT-9&z=1"), "DT-9");
        assert_eq!(parse_scanned_code("https://example.com/no-asset"), "https://example.com/no-asset");
    }
}
