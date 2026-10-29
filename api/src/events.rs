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
            // Nobody hears about an item outside their access.
            if let Some(asset) = e.asset {
                let sees: bool = Q::new("SELECT member_sees_asset($1, $2, $3)")
                    .bind(org)
                    .bind(e.user)
                    .bind(asset)
                    .scalar(&mut tx)
                    .await?;
                if !sees {
                    continue;
                }
            }
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

/// Tells reviewers (managers and admins who see the audit's inventory) that an audit awaits review.
pub async fn audit_submitted(state: &AppState, org: Uuid, run: Uuid) {
    let result: Result<Vec<Event>> = async {
        let mut db = state.pool.acquire().await?;
        let r = Q::new(
            "SELECT r.name, r.started_by, r.submitted_by, r.inventory_id, u.name AS submitter,
                    (SELECT count(*) FROM audit_items i WHERE i.run_id = r.id AND i.result = 'missing') AS missing
               FROM audit_runs r LEFT JOIN users u ON u.id = r.submitted_by WHERE r.id = $1",
        )
        .bind(run)
        .one(&mut db)
        .await?;
        let reviewers: Vec<Uuid> = Q::new(
            "SELECT m.user_id FROM memberships m
              WHERE m.org_id = $1 AND m.is_active AND m.user_id = ANY($2)
                AND (m.access_scope = 'all' OR ($3::uuid IS NOT NULL AND EXISTS (
                      WITH RECURSIVE t AS (
                        SELECT g.inventory_id AS id, 0 AS depth FROM inventory_grants g WHERE g.org_id = m.org_id AND g.user_id = m.user_id
                        UNION SELECT c.id, t.depth + 1 FROM inventories c JOIN t ON c.parent_id = t.id WHERE t.depth < 20)
                      SELECT 1 FROM t WHERE t.id = $3)))",
        )
        .bind(org)
        .bind(members_with(state, org, "audits:approve").await?)
        .bind(uuid_of(&r["inventory_id"]))
        .scalars(&mut db)
        .await?;
        let authors = [uuid_of(&r["started_by"]), uuid_of(&r["submitted_by"])];
        Ok(reviewers
            .into_iter()
            .filter(|u| !authors.contains(&Some(*u)))
            .map(|user| Event {
                user,
                kind: "audit_submitted",
                title: format!("Audit to review: {}", r["name"].as_str().unwrap_or("")),
                body: format!(
                    "{} submitted {} ({} missing).",
                    r["submitter"].as_str().unwrap_or(""),
                    r["name"].as_str().unwrap_or(""),
                    r["missing"]
                ),
                asset: None,
                link: Some("/?page=Approvals".into()),
                key: format!("audit-submitted:{run}:{}", chrono::Utc::now().timestamp()),
            })
            .collect())
    }
    .await;
    match result {
        Ok(events) => send_all(state, org, events).await,
        Err(e) => eprintln!("notify audit submitted: {e:?}"),
    }
}

/// Tells whoever submitted an audit how it was reviewed.
pub async fn audit_decided(state: &AppState, org: Uuid, run: Uuid) {
    let result: Result<Vec<Event>> = async {
        let mut db = state.pool.acquire().await?;
        let r = Q::new(
            "SELECT r.name, r.status, r.submitted_by, r.started_by, r.review_note, u.name AS reviewer, r.reviewed_at
               FROM audit_runs r LEFT JOIN users u ON u.id = r.reviewed_by WHERE r.id = $1",
        )
        .bind(run)
        .one(&mut db)
        .await?;
        let Some(user) = uuid_of(&r["submitted_by"]).or(uuid_of(&r["started_by"])) else { return Ok(vec![]) };
        let status = r["status"].as_str().unwrap_or("");
        let note = r["review_note"].as_str().map(|n| format!(" Note: {n}")).unwrap_or_default();
        Ok(vec![Event {
            user,
            kind: if status == "approved" { "audit_approved" } else { "audit_rejected" },
            title: format!("Audit {status}: {}", r["name"].as_str().unwrap_or("")),
            body: format!(
                "{} {status} {}.{note}",
                r["reviewer"].as_str().unwrap_or("A manager"),
                r["name"].as_str().unwrap_or("")
            ),
            asset: None,
            link: Some("/?page=Audits".into()),
            key: format!("audit-decided:{run}:{}", r["reviewed_at"].as_str().unwrap_or("")),
        }])
    }
    .await;
    match result {
        Ok(events) => send_all(state, org, events).await,
        Err(e) => eprintln!("notify audit decision: {e:?}"),
    }
}

/// The people who look after an item: its owner (a member) and its inventory's custodian.
async fn keepers(state: &AppState, asset: Uuid) -> Result<Vec<Uuid>> {
    let mut db = state.pool.acquire().await?;
    Q::new(
        "SELECT DISTINCT x.id FROM assets a LEFT JOIN inventories i ON i.id = a.inventory_id,
                LATERAL (VALUES (a.owner_id), (i.custodian_id)) AS x(id)
          WHERE a.id = $1 AND x.id IS NOT NULL",
    )
    .bind(asset)
    .scalars(&mut db)
    .await
}

/// An item was assigned to or returned from a person (outside a loan).
pub async fn custody_changed(state: &AppState, org: Uuid, asset: Uuid, actor: Uuid, assigned: bool) {
    let result: Result<Vec<Event>> = async {
        let mut db = state.pool.acquire().await?;
        let r = Q::new(
            "SELECT a.name, a.asset_tag, p.name AS person, u.name AS actor FROM assets a
               LEFT JOIN people p ON p.id = a.assigned_person_id JOIN users u ON u.id = $2 WHERE a.id = $1",
        )
        .bind(asset)
        .bind(actor)
        .one(&mut db)
        .await?;
        drop(db);
        let name = r["name"].as_str().unwrap_or("");
        let tag = r["asset_tag"].as_str().unwrap_or("");
        let stamp = chrono::Utc::now().timestamp_millis();
        Ok(keepers(state, asset)
            .await?
            .into_iter()
            .filter(|u| *u != actor)
            .map(|user| Event {
                user,
                kind: if assigned { "item_assigned" } else { "item_returned" },
                title: if assigned { format!("Assigned: {name}") } else { format!("Returned: {name}") },
                body: if assigned {
                    format!(
                        "{} assigned {name} ({tag}) to {}.",
                        r["actor"].as_str().unwrap_or(""),
                        r["person"].as_str().unwrap_or("")
                    )
                } else {
                    format!("{} recorded {name} ({tag}) as returned.", r["actor"].as_str().unwrap_or(""))
                },
                asset: Some(asset),
                link: Some(format!("/?asset={tag}")),
                key: format!("custody:{asset}:{stamp}"),
            })
            .collect())
    }
    .await;
    match result {
        Ok(events) => send_all(state, org, events).await,
        Err(e) => eprintln!("notify custody: {e:?}"),
    }
}

/// A loaner came back: tell whoever reserved it and the item's keepers.
pub async fn loan_returned(state: &AppState, org: Uuid, loan: Uuid, actor: Uuid) {
    let result: Result<Vec<Event>> = async {
        let mut db = state.pool.acquire().await?;
        let r = Q::new(
            "SELECT l.number, l.asset_id, l.reserved_by, l.damage_reported, a.name, a.asset_tag, p.name AS person
               FROM loans l JOIN assets a ON a.id = l.asset_id JOIN people p ON p.id = l.person_id WHERE l.id = $1",
        )
        .bind(loan)
        .one(&mut db)
        .await?;
        drop(db);
        let asset = uuid_of(&r["asset_id"]).unwrap();
        let mut to = keepers(state, asset).await?;
        to.extend(uuid_of(&r["reserved_by"]));
        to.sort();
        to.dedup();
        let damaged = if r["damage_reported"] == true { " It was reported damaged." } else { "" };
        Ok(to
            .into_iter()
            .filter(|u| *u != actor)
            .map(|user| Event {
                user,
                kind: "loan_returned",
                title: format!(
                    "Loan {} returned: {}",
                    r["number"].as_str().unwrap_or(""),
                    r["name"].as_str().unwrap_or("")
                ),
                body: format!(
                    "{} returned {} ({}). It needs an inspection before it is lent again.{damaged}",
                    r["person"].as_str().unwrap_or(""),
                    r["name"].as_str().unwrap_or(""),
                    r["asset_tag"].as_str().unwrap_or("")
                ),
                asset: Some(asset),
                link: Some("/?page=Loaners".into()),
                key: format!("loan-returned:{loan}"),
            })
            .collect())
    }
    .await;
    match result {
        Ok(events) => send_all(state, org, events).await,
        Err(e) => eprintln!("notify loan returned: {e:?}"),
    }
}

/// A finished audit left items missing: tell the audit inventory's custodian and the managers and
/// admins with access to everything.
pub async fn audit_missing(state: &AppState, org: Uuid, run: Uuid, actor: Uuid) {
    let result: Result<Vec<Event>> = async {
        let mut db = state.pool.acquire().await?;
        let r = Q::new(
            "SELECT r.name, i.custodian_id,
                    (SELECT count(*) FROM audit_items x WHERE x.run_id = r.id AND x.result = 'missing') AS missing
               FROM audit_runs r LEFT JOIN inventories i ON i.id = r.inventory_id WHERE r.id = $1",
        )
        .bind(run)
        .one(&mut db)
        .await?;
        let missing = r["missing"].as_i64().unwrap_or(0);
        if missing == 0 {
            return Ok(vec![]);
        }
        let mut to: Vec<Uuid> = Q::new(
            "SELECT user_id FROM memberships WHERE org_id = $1 AND is_active AND access_scope = 'all' AND user_id = ANY($2)",
        )
        .bind(org)
        .bind(members_with(state, org, "audits:approve").await?)
        .scalars(&mut db)
        .await?;
        to.extend(uuid_of(&r["custodian_id"]));
        to.sort();
        to.dedup();
        Ok(to
            .into_iter()
            .filter(|u| *u != actor)
            .map(|user| Event {
                user,
                kind: "audit_missing",
                title: format!("{missing} missing after {}", r["name"].as_str().unwrap_or("")),
                body: format!("{} finished with {missing} item(s) not found.", r["name"].as_str().unwrap_or("")),
                asset: None,
                link: Some("/?page=Approvals".into()),
                key: format!("audit-missing:{run}"),
            })
            .collect())
    }
    .await;
    match result {
        Ok(events) => send_all(state, org, events).await,
        Err(e) => eprintln!("notify audit missing: {e:?}"),
    }
}

/// Tells whoever sent an invitation that it was accepted.
pub async fn invitation_accepted(state: &AppState, org: Uuid, inviter: Option<Uuid>, member: Uuid) {
    let Some(inviter) = inviter.filter(|i| *i != member) else { return };
    let result: Result<Vec<Event>> = async {
        let mut db = state.pool.acquire().await?;
        let r = Q::new("SELECT u.name, u.email::text AS email, m.role FROM users u JOIN memberships m ON m.user_id = u.id AND m.org_id = $2 WHERE u.id = $1")
            .bind(member)
            .bind(org)
            .one(&mut db)
            .await?;
        Ok(vec![Event {
            user: inviter,
            kind: "invitation_accepted",
            title: format!("{} joined", r["name"].as_str().unwrap_or("")),
            body: format!(
                "{} ({}) accepted your invitation and joined as {}.",
                r["name"].as_str().unwrap_or(""),
                r["email"].as_str().unwrap_or(""),
                r["role"].as_str().unwrap_or("")
            ),
            asset: None,
            link: Some("/?page=Users".into()),
            key: format!("invitation-accepted:{org}:{member}"),
        }])
    }
    .await;
    match result {
        Ok(events) => send_all(state, org, events).await,
        Err(e) => eprintln!("notify invitation accepted: {e:?}"),
    }
}
