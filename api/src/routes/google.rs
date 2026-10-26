//! "Sign in with Google" (OpenID Connect authorization code flow with PKCE). Only offered when
//! GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET are set.
//!
//! - Accounts are matched by Google's stable subject id (`sub`), never by email.
//! - Each redirect carries a single-use state, also bound to the browser by a cookie, and a nonce
//!   checked inside the signed ID token, so forged or replayed callbacks are refused.
//! - Linking Google to an existing account requires being signed in to that account first.
//!   A Google login whose email matches an existing account does not sign in to it.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::header::{LOCATION, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, Scope, TokenResponse,
};
use serde_json::json;
use uuid::Uuid;

use crate::auth::{self, SignedIn};
use crate::config::{Config, GoogleConfig};
use crate::db::{uuid_of, Q};
use crate::error::{AppError, Result};
use crate::tokens;
use crate::AppState;

const STATE_COOKIE: &str = "dt_oauth";
const PROVIDER: &str = "google";

fn google(config: &Config) -> Result<&GoogleConfig> {
    config.google.as_ref().ok_or_else(|| AppError::coded(404, "google_unconfigured", "Google sign-in is not set up"))
}

fn http() -> Result<openidconnect::reqwest::Client> {
    // Following redirects would open the token exchange to request forgery.
    openidconnect::reqwest::ClientBuilder::new()
        .redirect(openidconnect::reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))
}

pub fn callback_url(config: &Config) -> String {
    format!("{}/api/auth/google/callback", config.app_url)
}

async fn client(
    config: &Config,
    http: &openidconnect::reqwest::Client,
) -> Result<
    CoreClient<
        openidconnect::EndpointSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointMaybeSet,
        openidconnect::EndpointMaybeSet,
    >,
> {
    let g = google(config)?;
    let issuer = IssuerUrl::new(g.issuer.clone()).map_err(|e| AppError::Internal(e.to_string()))?;
    let metadata = CoreProviderMetadata::discover_async(issuer, http)
        .await
        .map_err(|e| AppError::coded(502, "google_unavailable", format!("Could not reach Google: {e}")))?;
    let redirect = RedirectUrl::new(callback_url(config)).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(g.client_id.clone()),
        Some(ClientSecret::new(g.client_secret.clone())),
    )
    .set_redirect_uri(redirect))
}

fn state_cookie(config: &Config, value: &str, max_age: i64) -> String {
    let secure = if config.is_production { "; Secure" } else { "" };
    format!("{STATE_COOKIE}={value}; Path=/api/auth/google; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
}

/// Sends the browser to Google. `mode=link` adds Google to the signed-in account.
pub async fn start(
    State(state): State<AppState>,
    parts: Parts,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response> {
    google(&state.config)?;
    let mode = if q.get("mode").map(String::as_str) == Some("link") { "link" } else { "login" };
    let user_id = if mode == "link" {
        // Being signed in to the account is the proof of control needed to link it.
        let user = auth::authenticate(&parts, &state).await?;
        Some(user.id)
    } else {
        None
    };
    let http = http()?;
    let client = client(&state.config, &http).await?;
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, csrf, nonce) = client
        .authorize_url(CoreAuthenticationFlow::AuthorizationCode, CsrfToken::new_random, Nonce::new_random)
        .add_scope(Scope::new("email".into()))
        .add_scope(Scope::new("profile".into()))
        .set_pkce_challenge(challenge)
        .url();
    let mut db = state.pool.acquire().await?;
    Q::new("DELETE FROM oauth_states WHERE expires_at < now() - interval '1 day'").run(&mut db).await?;
    Q::new(
        "INSERT INTO oauth_states (state_hash, provider, mode, nonce, pkce_verifier, user_id, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now() + interval '10 minutes')",
    )
    .bind(tokens::hash(csrf.secret()))
    .bind(PROVIDER)
    .bind(mode)
    .bind(nonce.secret())
    .bind(verifier.secret())
    .bind(user_id)
    .run(&mut db)
    .await?;
    let cookie = state_cookie(&state.config, csrf.secret(), 600);
    Ok((StatusCode::FOUND, [(LOCATION, url.to_string()), (SET_COOKIE, cookie)]).into_response())
}

fn back_to_app(config: &Config, query: &str, extra_cookie: Option<String>) -> Response {
    let mut res = (StatusCode::FOUND, [(LOCATION, format!("{}/?{query}", config.app_url))]).into_response();
    let headers = res.headers_mut();
    headers.append(SET_COOKIE, state_cookie(config, "", 0).parse().unwrap());
    if let Some(c) = extra_cookie {
        headers.append(SET_COOKIE, c.parse().unwrap());
    }
    res
}

/// Google sends the browser back here with a one-time code.
pub async fn callback(
    State(state): State<AppState>,
    parts: Parts,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    match finish(&state, &parts, &q).await {
        Ok(res) => res,
        Err(code) => back_to_app(&state.config, &format!("google_error={code}"), None),
    }
}

fn internal<E: std::fmt::Debug>(e: E) -> &'static str {
    eprintln!("google sign-in failed: {e:?}");
    "unavailable"
}

async fn finish(
    state: &AppState,
    parts: &Parts,
    q: &HashMap<String, String>,
) -> std::result::Result<Response, &'static str> {
    let config = &state.config;
    if config.google.is_none() {
        return Err("unconfigured");
    }
    if q.contains_key("error") {
        return Err("cancelled");
    }
    let (Some(code), Some(csrf)) = (q.get("code"), q.get("state")) else { return Err("invalid") };
    // The state must be the one this browser started with, unused and recent.
    if auth::read_cookie(parts, STATE_COOKIE).as_deref() != Some(csrf.as_str()) {
        return Err("invalid");
    }
    let mut db = state.pool.acquire().await.map_err(internal)?;
    let saved = Q::new(
        "UPDATE oauth_states SET used_at = now()
          WHERE state_hash = $1 AND provider = $2 AND used_at IS NULL AND expires_at > now()
          RETURNING mode, nonce, pkce_verifier, user_id",
    )
    .bind(tokens::hash(csrf))
    .bind(PROVIDER)
    .opt(&mut db)
    .await
    .map_err(internal)?
    .ok_or("invalid")?;

    let http = http().map_err(internal)?;
    let client = client(config, &http).await.map_err(internal)?;
    let tokens = client
        .exchange_code(AuthorizationCode::new(code.clone()))
        .map_err(internal)?
        .set_pkce_verifier(PkceCodeVerifier::new(saved["pkce_verifier"].as_str().unwrap_or("").into()))
        .request_async(&http)
        .await
        .map_err(|_| "invalid")?;
    let id_token = tokens.id_token().ok_or("invalid")?;
    // Checks the signature, issuer, audience, expiry and our nonce.
    let nonce = Nonce::new(saved["nonce"].as_str().unwrap_or("").into());
    let claims = id_token.claims(&client.id_token_verifier(), &nonce).map_err(|_| "invalid")?;
    let subject = claims.subject().as_str().to_string();
    let email = claims.email().map(|e| e.as_str().to_lowercase());
    let email_verified = claims.email_verified() == Some(true);
    let name = claims
        .name()
        .and_then(|n| n.get(None))
        .map(|n| n.as_str().to_string())
        .or_else(|| email.clone())
        .unwrap_or_else(|| "Google user".into());

    let linked: Option<Uuid> = Q::new("SELECT user_id FROM user_identities WHERE provider = $1 AND subject = $2")
        .bind(PROVIDER)
        .bind(&subject)
        .scalars(&mut db)
        .await
        .map_err(internal)?
        .into_iter()
        .next();

    if saved["mode"] == "link" {
        let user_id = uuid_of(&saved["user_id"]).ok_or("invalid")?;
        match linked {
            Some(owner) if owner == user_id => {}
            Some(_) => return Err("already_linked"),
            None => {
                let inserted = Q::new(
                    "INSERT INTO user_identities (provider, subject, user_id, email) VALUES ($1, $2, $3, $4::citext)
                     ON CONFLICT DO NOTHING",
                )
                .bind(PROVIDER)
                .bind(&subject)
                .bind(user_id)
                .bind(&email)
                .run(&mut db)
                .await
                .map_err(internal)?;
                if inserted == 0 {
                    return Err("already_linked");
                }
            }
        }
        return Ok(back_to_app(config, "google=linked", None));
    }

    let user_id = match linked {
        Some(id) => id,
        None => {
            // A Google account is never attached to an existing account by matching email.
            let Some(email) = email.filter(|_| email_verified) else { return Err("email_unverified") };
            let taken: i64 = Q::new("SELECT count(*) FROM users WHERE email = $1::citext")
                .bind(&email)
                .scalar(&mut db)
                .await
                .map_err(internal)?;
            if taken > 0 {
                return Err("link_required");
            }
            if !config.allow_signup {
                return Err("no_account");
            }
            let mut tx = state.pool.begin().await.map_err(internal)?;
            let id: Uuid = Q::new(
                "INSERT INTO users (email, name, email_verified_at) VALUES ($1::citext, $2, now()) RETURNING id",
            )
            .bind(&email)
            .bind(&name)
            .scalar(&mut tx)
            .await
            .map_err(internal)?;
            Q::new("INSERT INTO user_identities (provider, subject, user_id, email) VALUES ($1, $2, $3, $4::citext)")
                .bind(PROVIDER)
                .bind(&subject)
                .bind(id)
                .bind(&email)
                .run(&mut tx)
                .await
                .map_err(internal)?;
            crate::routes::auth::create_workspace(&mut tx, id, &format!("{name}'s things"), "personal")
                .await
                .map_err(internal)?;
            tx.commit().await.map_err(internal)?;
            id
        }
    };
    let row = Q::new(
        "SELECT m.org_id FROM memberships m JOIN users u ON u.id = m.user_id
          WHERE m.user_id = $1 AND m.is_active AND u.is_active
          ORDER BY m.last_used_at DESC NULLS LAST, m.created_at LIMIT 1",
    )
    .bind(user_id)
    .opt(&mut db)
    .await
    .map_err(internal)?
    .ok_or("no_account")?;
    let org = uuid_of(&row["org_id"]).ok_or("invalid")?;
    let version: i32 = Q::new("UPDATE users SET last_login_at = now() WHERE id = $1 RETURNING session_version")
        .bind(user_id)
        .scalar(&mut db)
        .await
        .map_err(internal)?;
    Q::new("UPDATE memberships SET last_used_at = now() WHERE org_id = $1 AND user_id = $2")
        .bind(org)
        .bind(user_id)
        .run(&mut db)
        .await
        .map_err(internal)?;
    let session = auth::session_cookie(config, &auth::sign_session(config, user_id, org, version));
    Ok(back_to_app(config, "google=signed_in", Some(session)))
}

/// Removes Google from the account, if it can still sign in with a password.
pub async fn unlink(State(state): State<AppState>, SignedIn(user): SignedIn) -> Result<Json<serde_json::Value>> {
    if !user.has_password {
        return Err(AppError::new(400, "Set a password first, or you would have no way to sign in"));
    }
    let mut db = state.pool.acquire().await?;
    Q::new("DELETE FROM user_identities WHERE user_id = $1 AND provider = $2")
        .bind(user.id)
        .bind(PROVIDER)
        .run(&mut db)
        .await?;
    Ok(Json(json!({ "googleLinked": false })))
}
