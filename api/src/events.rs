//! Notifications about things that happen in a workspace (a transfer waiting for approval, a
//! decision on it, ...). They are sent after the change is committed; a failure to notify is
//! logged and never undoes the change. Each notification has a dedupe key, so a retried request
//! does not notify twice.

use serde_json::json;
use uuid::Uuid;

use crate::db::{uuid_of, Q};
use crate::error::Result;
use crate::notify::{self, Notice};
use crate::AppState;

/// One notification to send: who, what kind, title, body, link, and its dedupe key.
pub struct Event {
    pub user: Uuid,
    pub kind: &'static str,
    pub title: String,
    pub body: String,
    pub asset: Option<Uuid>,
    pub link: Option<String>,
    pub key: String,
}

pub async fn send_all(state: &AppState, org: Uuid, events: Vec<Event>) {
    if events.is_empty() {
        return;
    }
    let result: Result<()> = async {
        let mut tx = state.pool.begin().await?;
        for e in &events {
            let notice = Notice {
                asset_id: e.asset,
                link: e.link.clone(),
                dedupe_key: Some(format!("{}:{}", e.key, e.user)),
                ..Notice::new(org, e.user, e.kind, &e.title, &e.body)
            };
            notify::send(&mut tx, &state.config, notice).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        eprintln!(
            "{}",
            json!({ "level": "error", "t": chrono::Utc::now().to_rfc3339(), "error": format!("notify: {e:?}") })
        );
    }
}

/// Active members holding a permission in a workspace.
pub async fn members_with(state: &AppState, org: Uuid, permission: &str) -> Result<Vec<Uuid>> {
    let roles: Vec<String> =
        crate::auth::ROLES.iter().filter(|r| crate::auth::can(r, permission)).map(|r| r.to_string()).collect();
    let mut db = state.pool.acquire().await?;
    Q::new(
        "SELECT m.user_id FROM memberships m JOIN users u ON u.id = m.user_id
          WHERE m.org_id = $1 AND m.is_active AND u.is_active AND m.role = ANY($2)",
    )
    .bind(org)
    .bind(roles)
    .scalars(&mut db)
    .await
}

/// Tells the people who can approve transfers that some are waiting.
pub async fn transfers_requested(state: &AppState, org: Uuid, pending: &[Uuid]) {
    if pending.is_empty() {
        return;
    }
    let result: Result<Vec<Event>> = async {
        let mut db = state.pool.acquire().await?;
        let rows = Q::new(
            "SELECT t.id, t.batch_id, t.asset_id, t.via_asset_id, a.name, a.asset_tag, i.name AS target, u.name AS requester, t.requested_by
               FROM inventory_transfers t JOIN assets a ON a.id = t.asset_id JOIN inventories i ON i.id = t.to_inventory_id
               JOIN users u ON u.id = t.requested_by
              WHERE t.id = ANY($1) AND t.via_asset_id IS NULL",
        )
        .bind(pending.to_vec())
        .all(&mut db)
        .await?;
        drop(db);
        let approvers = members_with(state, org, "transfers:approve").await?;
        let mut events = vec![];
        for r in rows {
            for user in approvers.iter().filter(|u| Some(**u) != uuid_of(&r["requested_by"])) {
                events.push(Event {
                    user: *user,
                    kind: "transfer_requested",
                    title: format!("Transfer to approve: {}", r["name"].as_str().unwrap_or("")),
                    body: format!(
                        "{} asked to move {} ({}) to {}.",
                        r["requester"].as_str().unwrap_or(""),
                        r["name"].as_str().unwrap_or(""),
                        r["asset_tag"].as_str().unwrap_or(""),
                        r["target"].as_str().unwrap_or("")
                    ),
                    asset: uuid_of(&r["asset_id"]),
                    link: Some("/?page=Approvals".into()),
                    key: format!("transfer-requested:{}", r["id"].as_str().unwrap_or("")),
                });
            }
        }
        Ok(events)
    }
    .await;
    match result {
        Ok(events) => send_all(state, org, events).await,
        Err(e) => eprintln!("notify transfers: {e:?}"),
    }
}

/// Tells the person who asked how their transfer request was decided.
pub async fn transfer_decided(state: &AppState, org: Uuid, transfer: Uuid) {
    let result: Result<Vec<Event>> = async {
        let mut db = state.pool.acquire().await?;
        let r = Q::new(
            "SELECT t.status, t.requested_by, t.decided_by, t.decision_note, t.asset_id, a.name, a.asset_tag, i.name AS target, u.name AS decider
               FROM inventory_transfers t JOIN assets a ON a.id = t.asset_id JOIN inventories i ON i.id = t.to_inventory_id
               LEFT JOIN users u ON u.id = t.decided_by WHERE t.id = $1",
        )
        .bind(transfer)
        .one(&mut db)
        .await?;
        let requester = uuid_of(&r["requested_by"]).unwrap();
        if Some(requester) == uuid_of(&r["decided_by"]) {
            return Ok(vec![]);
        }
        let status = r["status"].as_str().unwrap_or("");
        let what = if status == "completed" { "approved" } else { status };
        let note = r["decision_note"].as_str().map(|n| format!(" Note: {n}")).unwrap_or_default();
        Ok(vec![Event {
            user: requester,
            kind: if status == "completed" { "transfer_completed" } else { "transfer_rejected" },
            title: format!("Transfer {what}: {}", r["name"].as_str().unwrap_or("")),
            body: format!(
                "{} {what} moving {} ({}) to {}.{note}",
                r["decider"].as_str().unwrap_or("A manager"),
                r["name"].as_str().unwrap_or(""),
                r["asset_tag"].as_str().unwrap_or(""),
                r["target"].as_str().unwrap_or("")
            ),
            asset: uuid_of(&r["asset_id"]),
            link: r["asset_tag"].as_str().map(|t| format!("/?asset={t}")),
            key: format!("transfer-decided:{transfer}"),
        }])
    }
    .await;
    match result {
        Ok(events) => send_all(state, org, events).await,
        Err(e) => eprintln!("notify transfer decision: {e:?}"),
    }
}

/// Tells the people who decide access requests (with access to everything) that one is waiting.
pub async fn access_requested(state: &AppState, org: Uuid, request: Uuid) {
    let result: Result<Vec<Event>> = async {
        let mut db = state.pool.acquire().await?;
        let r = Q::new(
            "SELECT r.user_id, u.name AS requester, i.name AS inventory FROM access_requests r
               JOIN users u ON u.id = r.user_id JOIN inventories i ON i.id = r.inventory_id WHERE r.id = $1",
        )
        .bind(request)
        .one(&mut db)
        .await?;
        let deciders: Vec<Uuid> = Q::new(
            "SELECT user_id FROM memberships WHERE org_id = $1 AND is_active AND access_scope = 'all' AND user_id = ANY($2)",
        )
        .bind(org)
        .bind(members_with(state, org, "access:approve").await?)
        .scalars(&mut db)
        .await?;
        let requester = uuid_of(&r["user_id"]);
        Ok(deciders
            .into_iter()
            .filter(|u| Some(*u) != requester)
            .map(|user| Event {
                user,
                kind: "access_requested",
                title: format!("Access request: {}", r["inventory"].as_str().unwrap_or("")),
                body: format!(
                    "{} asked for access to {}.",
                    r["requester"].as_str().unwrap_or(""),
                    r["inventory"].as_str().unwrap_or("")
                ),
                asset: None,
                link: Some("/?page=Approvals".into()),
                key: format!("access-requested:{request}"),
            })
            .collect())
    }
    .await;
    match result {
        Ok(events) => send_all(state, org, events).await,
        Err(e) => eprintln!("notify access request: {e:?}"),
    }
}

/// Tells the person who asked for access how it was decided.
pub async fn access_decided(state: &AppState, org: Uuid, request: Uuid) {
    let result: Result<Vec<Event>> = async {
        let mut db = state.pool.acquire().await?;
        let r = Q::new(
            "SELECT r.user_id, r.status, r.decision_note, i.name AS inventory, d.name AS decider FROM access_requests r
               JOIN inventories i ON i.id = r.inventory_id LEFT JOIN users d ON d.id = r.decided_by WHERE r.id = $1",
        )
        .bind(request)
        .one(&mut db)
        .await?;
        let status = r["status"].as_str().unwrap_or("");
        let note = r["decision_note"].as_str().map(|n| format!(" Note: {n}")).unwrap_or_default();
        Ok(vec![Event {
            user: uuid_of(&r["user_id"]).unwrap(),
            kind: if status == "approved" { "access_approved" } else { "access_rejected" },
            title: format!("Access {status}: {}", r["inventory"].as_str().unwrap_or("")),
            body: format!(
                "{} {status} your request for access to {}.{note}",
                r["decider"].as_str().unwrap_or("A manager"),
                r["inventory"].as_str().unwrap_or("")
            ),
            asset: None,
            link: Some("/?page=Inventories".into()),
            key: format!("access-decided:{request}"),
        }])
    }
    .await;
    match result {
        Ok(events) => send_all(state, org, events).await,
        Err(e) => eprintln!("notify access decision: {e:?}"),
    }
}
