use std::collections::{HashMap, HashSet};

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::auth::User;
use crate::db::Q;
use crate::error::{AppError, Result};
use crate::routes::assets::{record_event, ASSET_STATUSES};
use crate::validate::{is_date, Input};
use crate::web::Body;
use crate::AppState;

const MAX_ROWS: usize = 2000;

#[derive(Default)]
struct Row {
    name: String,
    kind: String,
    serial: Option<String>,
    manufacturer: Option<String>,
    model: Option<String>,
    status: Option<String>,
    building: Option<String>,
    location: Option<String>,
    purchase_date: Option<String>,
    warranty_expires: Option<String>,
    notes: Option<String>,
}

/// How a spreadsheet cell reads as text.
fn cell(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => Some(a.iter().map(|x| cell(Some(x)).unwrap_or_default()).collect::<Vec<_>>().join(",")),
        Value::Object(_) => Some("[object Object]".into()),
        other => Some(other.to_string()),
    }
}

/// Validates one row; returns it or the problems with it.
/// `categories` are the workspace's category names.
fn parse_row(raw: &Map<String, Value>, categories: &[&str]) -> std::result::Result<Row, Vec<String>> {
    let mut messages = vec![];
    let mut text = |key: &str, max: usize| -> Option<String> {
        let v = cell(raw.get(key))?.trim().to_string();
        if v.chars().count() > max {
            messages.push(format!("{key}: String must contain at most {max} character(s)"));
            return None;
        }
        (!v.is_empty()).then_some(v)
    };
    let name = text("name", 160);
    let serial = text("serial", 120);
    let manufacturer = text("manufacturer", 120);
    let model = text("model", 120);
    let building = text("building", 120);
    let location = text("location", 120);
    let notes = text("notes", 2000);
    if name.is_none() && !messages.iter().any(|m| m.starts_with("name:")) {
        messages.insert(0, "name: Required".into());
    }
    let mut pick = |key: &str, list: &[&str], required: bool| -> Option<String> {
        let v = cell(raw.get(key)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        match v {
            None if required => {
                messages.push(format!("{key}: Required"));
                None
            }
            None => None,
            Some(v) => match list.iter().find(|x| x.eq_ignore_ascii_case(&v)) {
                Some(x) => Some(x.to_string()),
                None => {
                    messages.push(format!("{key}: \"{v}\" is not one of: {}", list.join(", ")));
                    None
                }
            },
        }
    };
    let kind = pick("type", categories, true);
    let status = pick("status", ASSET_STATUSES, false);
    let mut date = |key: &str| -> Option<String> {
        let v = cell(raw.get(key)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())?;
        if !is_date(&v) {
            messages.push(format!("{key}: Use YYYY-MM-DD"));
            return None;
        }
        Some(v)
    };
    let purchase_date = date("purchaseDate");
    let warranty_expires = date("warrantyExpires");
    if !messages.is_empty() {
        return Err(messages);
    }
    Ok(Row {
        name: name.unwrap(),
        kind: kind.unwrap(),
        serial,
        manufacturer,
        model,
        status,
        building,
        location,
        purchase_date,
        warranty_expires,
        notes,
    })
}

/// Validates every row first; nothing is written unless all rows are valid, and then all are written together.
pub async fn import(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("assets:write")?;
    let mut input = Input::body(&body);
    let rows = match input.raw_value("rows") {
        None => {
            input.fail("rows", "Required");
            vec![]
        }
        Some(Value::Array(rows)) => {
            if rows.is_empty() {
                input.fail("rows", "The file has no rows");
            } else if rows.len() > MAX_ROWS {
                input.fail("rows", format!("Import at most {MAX_ROWS} rows at a time"));
            }
            for (i, r) in rows.iter().enumerate() {
                if !r.is_object() {
                    input.fail(
                        &format!("rows.{i}"),
                        format!("Expected object, received {}", crate::validate::type_name(r)),
                    );
                }
            }
            rows
        }
        Some(other) => {
            input.fail("rows", format!("Expected array, received {}", crate::validate::type_name(&other)));
            vec![]
        }
    };
    let create_sites = input.boolean_or("createSites", false);
    let dry_run = input.boolean_or("dryRun", true);
    input.strict()?;
    let org = user.org_id;
    let names = {
        let mut db = state.pool.acquire().await?;
        crate::routes::catalog::category_names(&mut db, org).await?
    };
    let categories: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut errors: Vec<(usize, Vec<String>)> = vec![];
    let parsed: Vec<Option<Row>> = rows
        .iter()
        .enumerate()
        .map(|(i, raw)| match parse_row(raw.as_object().unwrap(), &categories) {
            Err(messages) => {
                errors.push((i + 1, messages));
                None
            }
            Ok(row) => {
                if row.location.is_some() && row.building.is_none() {
                    errors.push((i + 1, vec!["location: needs a building".into()]));
                }
                Some(row)
            }
        })
        .collect();

    let mut tx = state.pool.begin().await?;
    let sites = Q::new(
        r#"SELECT b.id AS "buildingId", lower(b.name) AS building, l.id AS "locationId", lower(l.name) AS location
             FROM buildings b LEFT JOIN locations l ON l.building_id = b.id WHERE b.org_id = $1"#,
    )
    .bind(org)
    .all(&mut tx)
    .await?;
    let mut buildings: HashMap<String, Uuid> = HashMap::new();
    let mut locations: HashMap<String, Uuid> = HashMap::new();
    for s in &sites {
        let b = s["building"].as_str().unwrap_or("").to_string();
        buildings.insert(b.clone(), crate::db::uuid_of(&s["buildingId"]).unwrap());
        if let (Some(l), Some(id)) = (s["location"].as_str(), crate::db::uuid_of(&s["locationId"])) {
            locations.insert(format!("{b}/{l}"), id);
        }
    }
    let mut serials: HashSet<String> =
        Q::new("SELECT lower(serial) FROM assets WHERE org_id = $1 AND serial IS NOT NULL")
            .bind(org)
            .scalars(&mut tx)
            .await?
            .into_iter()
            .collect();
    // New sites in first-seen order, keeping the spelling of their first mention.
    let mut new_buildings: Vec<String> = vec![];
    let mut new_locations: Vec<(String, String)> = vec![];

    for (i, row) in parsed.iter().enumerate() {
        let Some(row) = row else { continue };
        let mut messages = vec![];
        if let Some(serial) = &row.serial {
            let key = serial.to_lowercase();
            if serials.contains(&key) {
                messages.push(format!("serial: {serial} already exists"));
            }
            serials.insert(key);
        }
        if let Some(building) = &row.building {
            let b = building.to_lowercase();
            if !buildings.contains_key(&b) {
                if create_sites {
                    if !new_buildings.contains(building) {
                        new_buildings.push(building.clone());
                    }
                } else {
                    messages.push(format!("building: \"{building}\" does not exist"));
                }
            }
            if let Some(location) = &row.location {
                if !locations.contains_key(&format!("{b}/{}", location.to_lowercase())) {
                    if create_sites {
                        let pair = (building.clone(), location.clone());
                        if !new_locations.contains(&pair) {
                            new_locations.push(pair);
                        }
                    } else {
                        messages.push(format!("location: \"{location}\" does not exist in {building}"));
                    }
                }
            }
        }
        if !messages.is_empty() {
            errors.push((i + 1, messages));
        }
    }
    errors.sort_by_key(|(row, _)| *row);
    let error_rows: HashSet<usize> = errors.iter().map(|(r, _)| *r).collect();
    let error_json: Vec<Value> =
        errors.iter().map(|(row, messages)| json!({ "row": row, "messages": messages })).collect();
    let mut summary = json!({
        "rows": rows.len(),
        "valid": rows.len() - error_rows.len(),
        "errors": error_json,
        "newBuildings": new_buildings,
        "newLocations": new_locations.iter().map(|(b, l)| format!("{b} / {l}")).collect::<Vec<_>>(),
        "created": 0,
    });

    if !dry_run && errors.is_empty() {
        for name in &new_buildings {
            let id: Uuid = Q::new("INSERT INTO buildings (org_id, name) VALUES ($1, $2) RETURNING id")
                .bind(org)
                .bind(name)
                .scalar(&mut tx)
                .await?;
            buildings.insert(name.to_lowercase(), id);
        }
        for (building, location) in &new_locations {
            let id: Uuid = Q::new("INSERT INTO locations (org_id, building_id, name) VALUES ($1, $2, $3) RETURNING id")
                .bind(org)
                .bind(buildings[&building.to_lowercase()])
                .bind(location)
                .scalar(&mut tx)
                .await?;
            locations.insert(format!("{}/{}", building.to_lowercase(), location.to_lowercase()), id);
        }
        crate::routes::plans::check_limit_by(&mut tx, org, "items", parsed.len() as i64).await?;
        let first: i32 = Q::new(
            "UPDATE organizations SET next_asset_number = next_asset_number + $2 WHERE id = $1 RETURNING next_asset_number - $2",
        )
        .bind(org)
        .bind(parsed.len() as i32)
        .scalar(&mut tx)
        .await?;
        for (i, row) in parsed.iter().enumerate() {
            let row = row.as_ref().unwrap();
            let building_id = row.building.as_ref().map(|b| buildings[&b.to_lowercase()]);
            let location_id = row
                .location
                .as_ref()
                .map(|l| locations[&format!("{}/{}", row.building.as_ref().unwrap().to_lowercase(), l.to_lowercase())]);
            let id: Uuid = Q::new(
                "INSERT INTO assets (org_id, asset_tag, name, type, serial, manufacturer, model, status, building_id, location_id,
                                     purchase_date, warranty_expires, notes, created_by)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::date, $12::date, $13, $14) RETURNING id",
            )
            .bind(org)
            .bind(format!("DT-{}", first + i as i32))
            .bind(&row.name)
            .bind(&row.kind)
            .bind(&row.serial)
            .bind(&row.manufacturer)
            .bind(&row.model)
            .bind(row.status.as_deref().unwrap_or("Available"))
            .bind(building_id)
            .bind(location_id)
            .bind(&row.purchase_date)
            .bind(&row.warranty_expires)
            .bind(&row.notes)
            .bind(user.id)
            .scalar(&mut tx)
            .await?;
            if let Some(date) = &row.warranty_expires {
                crate::routes::warranties::set_primary(&mut tx, org, id, Some(date)).await?;
            }
            let changes = json!({ "source": { "from": null, "to": "csv import" } });
            record_event(&mut tx, org, id, user.id, "created", changes).await?;
        }
        summary["created"] = json!(parsed.len());
    }
    tx.commit().await?;
    if !dry_run && !errors.is_empty() {
        return Err(AppError::with_details(
            400,
            format!("{} row(s) need fixing before anything is imported", errors.len()),
            json!(error_json),
        ));
    }
    Ok((if dry_run { StatusCode::OK } else { StatusCode::CREATED }, Json(summary)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATEGORIES: &[&str] = &["Laptop", "Monitor"];

    #[test]
    fn rows_are_matched_case_insensitively_and_explained() {
        let ok = parse_row(
            json!({ "name": " Dell ", "type": "laptop", "status": "deployed", "serial": 7 }).as_object().unwrap(),
            CATEGORIES,
        )
        .unwrap();
        assert_eq!((ok.name.as_str(), ok.kind.as_str(), ok.status.as_deref()), ("Dell", "Laptop", Some("Deployed")));
        assert_eq!(ok.serial.as_deref(), Some("7"));
        let bad = parse_row(
            json!({ "name": "", "type": "Toaster", "purchaseDate": "1/2/2024" }).as_object().unwrap(),
            CATEGORIES,
        )
        .err()
        .unwrap();
        assert_eq!(bad[0], "name: Required");
        assert!(bad[1].starts_with("type: \"Toaster\" is not one of: Laptop"));
        assert_eq!(bad[2], "purchaseDate: Use YYYY-MM-DD");
    }
}
