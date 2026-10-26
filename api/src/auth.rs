use std::sync::{Arc, Mutex, OnceLock};

use axum::extract::FromRequestParts;
use axum::http::header::{AUTHORIZATION, COOKIE};
use axum::http::request::Parts;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::Config;
use crate::db::Q;
use crate::error::{AppError, Result};
use crate::AppState;

pub const SESSION_COOKIE: &str = "dt_session";
const BCRYPT_COST: u32 = 12;

pub const ROLES: &[&str] = &["admin", "manager", "technician", "auditor", "viewer"];

/// Which roles hold each permission. New features add their permissions here.
const GRANTS: &[(&str, &[&str])] = &[
    ("assets:read", &["admin", "manager", "technician", "auditor", "viewer"]),
    ("assets:write", &["admin", "manager", "technician"]),
    ("sites:read", &["admin", "manager", "technician", "auditor", "viewer"]),
    ("sites:write", &["admin", "manager"]),
    ("people:write", &["admin", "manager", "technician"]),
    ("audits:read", &["admin", "manager", "technician", "auditor", "viewer"]),
    ("audits:perform", &["admin", "manager", "auditor"]),
    ("maintenance:read", &["admin", "manager", "technician", "auditor", "viewer"]),
    ("maintenance:write", &["admin", "manager", "technician"]),
    ("users:read", &["admin", "manager"]),
    ("users:manage", &["admin"]),
    ("settings:manage", &["admin"]),
    ("catalog:manage", &["admin", "manager"]),
    ("devices:manage", &["admin", "manager"]),
];

pub fn can(role: &str, permission: &str) -> bool {
    GRANTS.iter().any(|(p, roles)| *p == permission && roles.contains(&role))
}

pub fn permissions_for(role: &str) -> Vec<&'static str> {
    GRANTS.iter().filter(|(_, roles)| roles.contains(&role)).map(|(p, _)| *p).collect()
}

pub async fn hash_password(password: String) -> Result<String> {
    tokio::task::spawn_blocking(move || bcrypt::hash(password, BCRYPT_COST))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(|e| AppError::Internal(e.to_string()))
}

pub async fn verify_password(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || bcrypt::verify(password, &hash).unwrap_or(false)).await.unwrap_or(false)
}

/// Compared against when an email is unknown, so login timing does not reveal which emails exist.
pub fn dummy_hash() -> String {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| bcrypt::hash("not-a-real-password", BCRYPT_COST).unwrap()).clone()
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    org: String,
    ver: i32,
    iat: i64,
    exp: i64,
}

pub fn sign_session(config: &Config, user_id: Uuid, org_id: Uuid, version: i32) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = Claims {
        sub: user_id.to_string(),
        org: org_id.to_string(),
        ver: version,
        iat: now,
        exp: now + config.session_hours * 3600,
    };
    jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
    )
    .expect("sign session")
}

fn verify_session(config: &Config, token: &str) -> Option<Claims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    validation.required_spec_claims = ["exp", "sub"].iter().map(|s| s.to_string()).collect();
    jsonwebtoken::decode::<Claims>(token, &DecodingKey::from_secret(config.jwt_secret.as_bytes()), &validation)
        .ok()
        .map(|d| d.claims)
}

fn cookie_attributes(config: &Config) -> String {
    format!("Path=/; HttpOnly; SameSite=Lax{}", if config.is_production { "; Secure" } else { "" })
}

pub fn session_cookie(config: &Config, token: &str) -> String {
    let max_age = config.session_hours * 3600;
    let expires = (chrono::Utc::now() + chrono::Duration::seconds(max_age)).format("%a, %d %b %Y %H:%M:%S GMT");
    format!("{SESSION_COOKIE}={token}; Max-Age={max_age}; Expires={expires}; {}", cookie_attributes(config))
}

pub fn clear_session_cookie(config: &Config) -> String {
    format!("{SESSION_COOKIE}=; Expires=Thu, 01 Jan 1970 00:00:00 GMT; {}", cookie_attributes(config))
}

pub fn read_cookie(parts: &Parts, name: &str) -> Option<String> {
    parts.headers.get_all(COOKIE).iter().filter_map(|h| h.to_str().ok()).flat_map(|h| h.split(';')).find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == name).then(|| v.trim_matches('"').to_string())
    })
}

/// Lets the request log see who made a request.
#[derive(Clone, Default)]
pub struct UserSlot(pub Arc<Mutex<Option<Uuid>>>);

/// The signed-in user in their current workspace, loaded fresh on every request so deactivation
/// and role changes apply at once.
#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub org_id: Uuid,
    pub email: String,
    pub name: String,
    pub role: String,
    pub org_name: String,
    pub org_kind: String,
    pub email_verified: bool,
    pub has_password: bool,
}

/// The same, but also for accounts that have not confirmed their email yet. Used by the few
/// routes such a person needs (who am I, confirm email, sign out, switch workspace).
pub struct SignedIn(pub User);

pub const USER_SELECT: &str = "SELECT u.id, m.org_id, u.email, u.name, m.role, o.name AS org_name, o.kind AS org_kind,
        u.email_verified_at IS NOT NULL AS email_verified, u.password_hash IS NOT NULL AS has_password
   FROM users u JOIN memberships m ON m.user_id = u.id JOIN organizations o ON o.id = m.org_id";

impl User {
    pub fn require(&self, permission: &str) -> Result<()> {
        if can(&self.role, permission) {
            Ok(())
        } else {
            Err(AppError::new(403, "You do not have permission to do that"))
        }
    }

    pub fn public(&self) -> Value {
        json!({
            "id": self.id,
            "email": self.email,
            "name": self.name,
            "role": self.role,
            "emailVerified": self.email_verified,
            "hasPassword": self.has_password,
            "organization": { "id": self.org_id, "name": self.org_name, "kind": self.org_kind },
            "permissions": permissions_for(&self.role),
        })
    }

    pub fn from_row(row: &Value) -> Option<User> {
        Some(User {
            id: crate::db::uuid_of(&row["id"])?,
            org_id: crate::db::uuid_of(&row["org_id"])?,
            email: row["email"].as_str()?.to_string(),
            name: row["name"].as_str()?.to_string(),
            role: row["role"].as_str()?.to_string(),
            org_name: row["org_name"].as_str()?.to_string(),
            org_kind: row["org_kind"].as_str().unwrap_or("business").to_string(),
            email_verified: row["email_verified"] == true,
            has_password: row["has_password"] != false,
        })
    }
}

/// Loads an account in one of its workspaces, if both the account and the membership are active.
pub async fn load_user(db: &mut crate::db::Db, user_id: Uuid, org_id: Uuid) -> Result<Option<User>> {
    let row = Q::new(format!("{USER_SELECT} WHERE u.id = $1 AND m.org_id = $2 AND u.is_active AND m.is_active"))
        .bind(user_id)
        .bind(org_id)
        .opt(db)
        .await?;
    Ok(row.as_ref().and_then(User::from_row))
}

pub async fn authenticate(parts: &Parts, state: &AppState) -> Result<User> {
    let token = parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::to_string)
        .or_else(|| read_cookie(parts, SESSION_COOKIE))
        .ok_or_else(|| AppError::new(401, "Sign in required"))?;
    let invalid = || AppError::new(401, "Session expired or invalid");
    let claims = verify_session(&state.config, &token).ok_or_else(invalid)?;
    let (Ok(id), Ok(org)) = (Uuid::parse_str(&claims.sub), Uuid::parse_str(&claims.org)) else {
        return Err(invalid());
    };
    let mut db = state.pool.acquire().await?;
    let version: Option<i32> =
        Q::new("SELECT session_version FROM users WHERE id = $1").bind(id).scalars(&mut db).await?.into_iter().next();
    if version != Some(claims.ver) {
        return Err(invalid());
    }
    let user = load_user(&mut db, id, org).await?.ok_or_else(invalid)?;
    if let Some(slot) = parts.extensions.get::<UserSlot>() {
        *slot.0.lock().unwrap() = Some(user.id);
    }
    Ok(user)
}

impl FromRequestParts<AppState> for User {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<User> {
        let user = authenticate(parts, state).await?;
        if state.config.require_email_verification && !user.email_verified {
            return Err(AppError::coded(
                403,
                "email_unverified",
                "Confirm your email address to continue. Check your inbox for the link.",
            ));
        }
        Ok(user)
    }
}

impl FromRequestParts<AppState> for SignedIn {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<SignedIn> {
        authenticate(parts, state).await.map(SignedIn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_get_exactly_their_grants() {
        assert!(can("technician", "maintenance:write"));
        assert!(!can("auditor", "assets:write"));
        assert!(can("auditor", "audits:perform"));
        assert!(!can("manager", "users:manage"));
        assert!(!can("nobody", "assets:read"));
        assert_eq!(permissions_for("viewer"), ["assets:read", "sites:read", "audits:read", "maintenance:read"]);
    }

    #[test]
    fn sessions_round_trip_and_reject_tampering() {
        let config = Config::from_env().unwrap();
        let (u, o) = (Uuid::from_u128(7), Uuid::from_u128(9));
        let token = sign_session(&config, u, o, 3);
        let claims = verify_session(&config, &token).unwrap();
        assert_eq!((claims.sub, claims.org, claims.ver), (u.to_string(), o.to_string(), 3));
        assert!(verify_session(&config, &format!("{token}x")).is_none());
    }
}
