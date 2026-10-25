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

pub const ASSET_TYPES: &[&str] = &[
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
pub const ASSET_STATUSES: &[&str] = &["Available", "Deployed", "Maintenance", "Retired"];

/// API field, column, and the cast that types a text parameter for that column. Every editable field.
const COLUMNS: &[(&str, &str, &str)] = &[
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
];

pub const SELECT: &str = r#"
  SELECT a.id, a.asset_tag AS "assetTag", a.name, a.type, a.serial, a.manufacturer, a.model, a.status,
         a.building_id AS "buildingId", b.name AS "buildingName",
         a.location_id AS "locationId", l.name AS "locationName",
         a.parent_asset_id AS "parentId", pa.asset_tag AS "parentTag", pa.name AS "parentName",
         a.assigned_person_id AS "assignedPersonId", pe.name AS "assignedPersonName", a.assigned_at AS "assignedAt",
         (SELECT count(*)::int FROM assets c WHERE c.parent_asset_id = a.id) AS "childCount",
         a.notes, a.purchase_date AS "purchaseDate", a.warranty_expires AS "warrantyExpires",
         a.created_at AS "createdAt", a.updated_at AS "updatedAt"
    FROM assets a
    LEFT JOIN buildings b ON b.id = a.building_id
    LEFT JOIN locations l ON l.id = a.location_id
    LEFT JOIN assets pa ON pa.id = a.parent_asset_id
    LEFT JOIN people pe ON pe.id = a.assigned_person_id"#;

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

fn asset_fields(input: &mut Input, partial: bool) -> Fields {
    let name = if partial { Str::req(1, 160).optional() } else { Str::req(1, 160) };
    let mut f = vec![
        ("name", input.str("name", name)),
        ("type", input.one_of("type", ASSET_TYPES, partial, false)),
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
    ];
    if !partial && f[5].1.is_absent() {
        f[5].1 = Opt::Val("Available".into());
    }
    Fields(f)
}

pub async fn find_asset(db: &mut Db, org_id: Uuid, id: Uuid) -> Result<Value> {
    Q::new(format!("{SELECT} WHERE a.org_id = $1 AND a.id = $2"))
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
async fn descendant_ids(db: &mut Db, org_id: Uuid, id: Uuid) -> Result<Vec<Uuid>> {
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
    let mut input = Input::query(&query);
    let q = input.str("q", Str::req(0, 200).optional());
    let status = input.one_of("status", ASSET_STATUSES, true, false);
    let kind = input.one_of("type", ASSET_TYPES, true, false);
    let building = input.uuid("buildingId", true, false);
    let location = input.uuid("locationId", true, false);
    let person = input.uuid("personId", true, false);
    let parent = input.uuid("parentId", true, false);
    let limit = input.int("limit", 1, 500, 100);
    let offset = input.int("offset", 0, i64::MAX, 0);
    input.done()?;

    let mut wheres = vec!["a.org_id = $1".to_string()];
    let mut text_params: Vec<String> = vec![];
    let mut uuid_params: Vec<Uuid> = vec![];
    // Text parameters come first ($2, $3...), then UUIDs, so each keeps its own type.
    let mut text = |sql: &str, v: String, wheres: &mut Vec<String>| {
        text_params.push(v);
        wheres.push(sql.replace('?', &format!("${}", text_params.len() + 1)));
    };
    if let Some(q) = q.into_option().filter(|q| !q.is_empty()) {
        let escaped: String =
            q.chars().flat_map(|c| if "\\%_".contains(c) { vec!['\\', c] } else { vec![c] }).collect();
        text(
            "(a.asset_tag ILIKE ? OR a.name ILIKE ? OR a.serial ILIKE ? OR a.type ILIKE ?
              OR a.model ILIKE ? OR b.name ILIKE ? OR l.name ILIKE ? OR pe.name ILIKE ?)",
            format!("%{escaped}%"),
            &mut wheres,
        );
    }
    if let Opt::Val(s) = status {
        text("a.status = ?", s, &mut wheres);
    }
    if let Opt::Val(t) = kind {
        text("a.type = ?", t, &mut wheres);
    }
    let first_uuid = text_params.len() + 2;
    for (column, value) in [
        ("a.building_id", building),
        ("a.location_id", location),
        ("a.assigned_person_id", person),
        ("a.parent_asset_id", parent),
    ] {
        if let Opt::Val(u) = value {
            wheres.push(format!("{column} = ${}", first_uuid + uuid_params.len()));
            uuid_params.push(u);
        }
    }
    let where_sql = wheres.join(" AND ");
    let build = |sql: String| {
        let mut q = Q::new(sql).bind(user.org_id);
        for t in &text_params {
            q = q.bind(t.clone());
        }
        for u in &uuid_params {
            q = q.bind(*u);
        }
        q
    };
    let mut db = state.pool.acquire().await?;
    let items = build(format!(
        "{SELECT} WHERE {where_sql} ORDER BY a.created_at DESC, a.asset_tag DESC LIMIT {limit} OFFSET {offset}"
    ))
    .all(&mut db)
    .await?;
    let total: i32 = build(format!(
        "SELECT count(*)::int AS total FROM assets a
           LEFT JOIN buildings b ON b.id = a.building_id
           LEFT JOIN locations l ON l.id = a.location_id
           LEFT JOIN people pe ON pe.id = a.assigned_person_id WHERE {where_sql}"
    ))
    .scalar(&mut db)
    .await?;
    Ok(Json(json!({ "items": items, "total": total, "limit": limit, "offset": offset })))
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
        "{SELECT} WHERE a.org_id = $1 AND (lower(a.asset_tag) = lower($2) OR lower(a.serial) = lower($2))
          ORDER BY (lower(a.asset_tag) = lower($2)) DESC LIMIT 1"
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
    Ok(Json(find_by_code(&mut db, user.org_id, &code).await?))
}

pub async fn get(State(state): State<AppState>, user: User, Path(id): Path<String>) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let id = path_uuid(&id)?;
    let mut db = state.pool.acquire().await?;
    let mut asset = find_asset(&mut db, user.org_id, id).await?;
    let children = Q::new(format!("{SELECT} WHERE a.org_id = $1 AND a.parent_asset_id = $2 ORDER BY a.asset_tag"))
        .bind(user.org_id)
        .bind(id)
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
    input.strict()?;
    let org = user.org_id;

    let mut tx = state.pool.begin().await?;
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
            .bind(format!("DT-{n}"))
            .bind(user.id);
    for p in q_params {
        q = q.bind(p);
    }
    let id: Uuid = q.scalar(&mut tx).await?;
    let created = find_asset(&mut tx, org, id).await?;
    let changes: Map<String, Value> =
        present.iter().map(|(k, v)| (k.to_string(), json!({ "from": null, "to": v }))).collect();
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
    input.strict()?;
    let org = user.org_id;

    let mut tx = state.pool.begin().await?;
    let current = Q::new(
        r#"SELECT name, type, serial, manufacturer, model, status, building_id AS "buildingId",
                  location_id AS "locationId", parent_asset_id AS "parentId", notes,
                  purchase_date AS "purchaseDate", warranty_expires AS "warrantyExpires"
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
    let mut assign = Assign::new(3);
    for (key, to) in fields.present() {
        if to != current[key] {
            let (_, column, cast) = COLUMNS.iter().find(|(k, _, _)| *k == key).unwrap();
            assign.set(column, cast, Param::Text(to.as_str().map(str::to_string)));
            changes.insert(key.to_string(), json!({ "from": current[key], "to": to }));
        }
    }
    let moved = changes.contains_key("buildingId") || changes.contains_key("locationId");
    if !changes.is_empty() {
        let sql = format!("UPDATE assets SET {}, updated_at = now() WHERE org_id = $1 AND id = $2", assign.sql());
        assign.bind_to(Q::new(sql).bind(org).bind(id)).run(&mut tx).await?;
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
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let person = find_person(&mut tx, user.org_id, person_id.into_option().unwrap()).await?;
    set_assignment(&mut tx, &user, id, Some(&person), "", include, note).await?;
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
