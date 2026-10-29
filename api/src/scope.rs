//! Server-side enforcement of inventory-limited access. People whose access is limited to some
//! inventories reach an item (and its files, warranties, work orders and transfers) or an
//! inventory only when it lies inside what they were granted; anything else answers 404, exactly
//! as for something in another workspace. Workspace-wide views and settings are refused to them.
//! Lists filter by the same scope in their own queries (`User::scope_sql`).

use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::db::Q;
use crate::error::AppError;
use crate::AppState;

/// Paths whose second segment is an id, and the query that finds the inventory (or inventories)
/// that id belongs to. Each query takes ($1 id, $2 workspace) and returns inventory ids.
const OBJECTS: &[(&str, &str, &str)] = &[
    ("assets", "Asset", "SELECT inventory_id FROM assets WHERE id = $1 AND org_id = $2"),
    (
        "attachments",
        "Attachment",
        "SELECT a.inventory_id FROM attachments f JOIN assets a ON a.id = f.asset_id WHERE f.id = $1 AND f.org_id = $2",
    ),
    (
        "warranties",
        "Warranty",
        "SELECT a.inventory_id FROM warranties w JOIN assets a ON a.id = w.asset_id WHERE w.id = $1 AND w.org_id = $2",
    ),
    (
        "work-orders",
        "Work order",
        "SELECT a.inventory_id FROM work_orders w JOIN assets a ON a.id = w.asset_id WHERE w.id = $1 AND w.org_id = $2",
    ),
    (
        "transfers",
        "Transfer",
        "SELECT unnest(ARRAY[t.to_inventory_id, t.from_inventory_id]) FROM inventory_transfers t WHERE t.id = $1 AND t.org_id = $2",
    ),
    ("inventories", "Inventory", "SELECT id FROM inventories WHERE id = $1 AND org_id = $2"),
    ("audits", "Audit", "SELECT inventory_id FROM audit_runs WHERE id = $1 AND org_id = $2"),
    (
        "loans",
        "Loan",
        "SELECT a.inventory_id FROM loans l JOIN assets a ON a.id = l.asset_id WHERE l.id = $1 AND l.org_id = $2",
    ),
];

/// Workspace-wide areas closed to people with limited access (reads and writes).
const CLOSED: &[&str] = &["reports", "insights", "agents", "discovered-devices"];
/// Workspace-wide areas they may read but not change.
const READ_ONLY: &[&str] = &["categories", "tags", "custom-fields", "buildings", "locations", "people", "settings"];

pub async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let parts: Vec<&str> = path.trim_start_matches("/api/").split('/').collect();
    let area = parts.first().copied().unwrap_or("");
    let id = parts.get(1).and_then(|s| crate::validate::parse_uuid(s));
    let object = OBJECTS.iter().find(|(a, _, _)| *a == area);
    let writes = ![Method::GET, Method::HEAD, Method::OPTIONS].contains(req.method());
    let concerned = (object.is_some() && id.is_some())
        || CLOSED.contains(&area)
        || (writes && READ_ONLY.contains(&area))
        || (area == "assets" && parts.get(1) == Some(&"import"));
    if !concerned || !path.starts_with("/api/") {
        return next.run(req).await;
    }
    let (parts_req, body) = req.into_parts();
    // Not signed in, or a session problem: the route itself answers that.
    let Ok(user) = crate::auth::authenticate(&parts_req, &state).await else {
        return next.run(Request::from_parts(parts_req, body)).await;
    };
    if user.scope.is_none() {
        return next.run(Request::from_parts(parts_req, body)).await;
    }
    if CLOSED.contains(&area) || READ_ONLY.contains(&area) || area == "assets" && id.is_none() {
        return AppError::new(403, "Your access is limited to selected inventories").into_response();
    }
    let (_, what, sql) = object.unwrap();
    let inventories: Result<Vec<Option<Uuid>>, AppError> = async {
        let mut db = state.pool.acquire().await?;
        Q::new(*sql).bind(id.unwrap()).bind(user.org_id).scalars(&mut db).await
    }
    .await;
    match inventories {
        Err(e) => e.into_response(),
        Ok(found) if found.iter().any(|i| user.sees_inventory(*i)) => {
            next.run(Request::from_parts(parts_req, body)).await
        }
        // Not found at all, or outside this person's access: the same answer.
        Ok(_) => AppError::not_found(what).into_response(),
    }
}
