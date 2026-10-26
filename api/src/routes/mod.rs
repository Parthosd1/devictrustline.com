pub mod assets;
pub mod audits;
pub mod auth;
pub mod import;
pub mod invitations;
pub mod people;
pub mod reports;
pub mod settings;
pub mod sites;
pub mod users;
pub mod work_orders;

use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde_json::json;

use crate::web;
use crate::AppState;

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "status": "ok" }))),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "status": "database unavailable" }))),
    }
}

pub fn api(state: &AppState) -> Router<AppState> {
    let limited = Router::new()
        .route("/api/auth/signup", post(auth::signup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/change-password", post(auth::change_password))
        .route("/api/auth/verify-email", post(auth::verify_email))
        .route("/api/auth/resend-verification", post(auth::resend_verification))
        .route("/api/auth/password-reset/request", post(auth::request_password_reset))
        .route("/api/auth/password-reset/confirm", post(auth::reset_password))
        .route("/api/invitations/accept", post(invitations::accept))
        .route_layer(from_fn_with_state(state.clone(), web::rate_limit));

    Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/config", get(auth::config))
        .route("/api/auth/switch", post(auth::switch))
        .route("/api/account", get(auth::account).patch(auth::update_account))
        .route("/api/workspaces", get(auth::workspaces).post(auth::create_workspace_route))
        .route("/api/invitations", get(invitations::list).post(invitations::create))
        .route("/api/invitations/lookup", get(invitations::lookup))
        .route("/api/invitations/{id}", axum::routing::delete(invitations::revoke))
        .route("/api/dev/mailbox", get(invitations::dev_mailbox))
        .merge(limited)
        .route("/api/users", get(users::list).post(users::create))
        .route("/api/users/{id}", patch(users::update))
        .route("/api/assets/import", post(import::import))
        .route("/api/assets", get(assets::list).post(assets::create))
        .route("/api/assets/lookup", get(assets::lookup))
        .route("/api/assets/{id}", get(assets::get).patch(assets::update))
        .route("/api/assets/{id}/events", get(assets::events))
        .route("/api/assets/{id}/assign", post(assets::assign))
        .route("/api/assets/{id}/return", post(assets::return_asset))
        .route("/api/people", get(people::list).post(people::create))
        .route("/api/people/{id}", patch(people::update))
        .route("/api/audits", get(audits::list).post(audits::start))
        .route("/api/audits/{id}", get(audits::get))
        .route("/api/audits/{id}/scan", post(audits::scan))
        .route("/api/audits/{id}/items/{asset_id}", patch(audits::mark_item))
        .route("/api/audits/{id}/close", post(audits::close))
        .route("/api/work-orders", get(work_orders::list).post(work_orders::create))
        .route("/api/work-orders/assignees", get(work_orders::assignees))
        .route("/api/work-orders/{id}", get(work_orders::get).patch(work_orders::update))
        .route("/api/work-orders/{id}/notes", post(work_orders::add_note_route))
        .route("/api/work-orders/{id}/resolve", post(work_orders::resolve))
        .route("/api/work-orders/{id}/cancel", post(work_orders::cancel))
        .route("/api/reports/summary", get(reports::summary))
        .route("/api/settings", get(settings::get).patch(settings::update))
        .route("/api/buildings", get(sites::list_buildings).post(sites::create_building))
        .route("/api/buildings/{id}", patch(sites::update_building).delete(sites::delete_building))
        .route("/api/buildings/{id}/locations", post(sites::create_location))
        .route("/api/locations/{id}", patch(sites::update_location).delete(sites::delete_location))
        .route("/api", axum::routing::any(web::api_not_found))
        .route("/api/{*rest}", axum::routing::any(web::api_not_found))
        .method_not_allowed_fallback(web::api_not_found)
}
