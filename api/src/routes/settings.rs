use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::auth::User;
use crate::db::Q;
use crate::error::Result;
use crate::validate::Input;
use crate::web::Body;
use crate::AppState;

/// An organization's settings with every default filled in.
pub fn with_defaults(settings: &Value) -> Value {
    let mut out = json!({ "autoAudits": { "weekly": false, "monthly": false } });
    if let Some(map) = settings.as_object() {
        for (k, v) in map {
            if k != "autoAudits" {
                out[k] = v.clone();
            }
        }
        if let Some(auto) = map.get("autoAudits").and_then(Value::as_object) {
            for (k, v) in auto {
                out["autoAudits"][k] = v.clone();
            }
        }
    }
    out
}

pub async fn get(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    let mut db = state.pool.acquire().await?;
    let settings: Value =
        Q::new("SELECT settings FROM organizations WHERE id = $1").bind(user.org_id).scalar(&mut db).await?;
    Ok(Json(with_defaults(&settings)))
}

pub async fn update(State(state): State<AppState>, user: User, Body(body): Body) -> Result<Json<Value>> {
    user.require("settings:manage")?;
    let mut input = Input::body(&body);
    let mut auto = (None, None);
    input.object("autoAudits", |a| {
        auto = (a.boolean("weekly", false).into_option(), a.boolean("monthly", false).into_option());
    });
    input.strict()?;
    let mut tx = state.pool.begin().await?;
    let settings: Value =
        Q::new("SELECT settings FROM organizations WHERE id = $1 FOR UPDATE").bind(user.org_id).scalar(&mut tx).await?;
    let mut next = with_defaults(&settings);
    if let Some(weekly) = auto.0 {
        next["autoAudits"]["weekly"] = json!(weekly);
    }
    if let Some(monthly) = auto.1 {
        next["autoAudits"]["monthly"] = json!(monthly);
    }
    Q::new("UPDATE organizations SET settings = $2 WHERE id = $1").bind(user.org_id).bind(&next).run(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(next))
}
