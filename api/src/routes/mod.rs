pub mod access_requests;
pub mod agents;
pub mod assets;
pub mod attachments;
pub mod audits;
pub mod auth;
pub mod catalog;
pub mod google;
pub mod import;
pub mod inventories;
pub mod invitations;
pub mod loans;
pub mod notifications;
pub mod people;
pub mod plans;
pub mod reports;
pub mod settings;
pub mod sites;
pub mod transfers;
pub mod user_reminders;
pub mod users;
pub mod warranties;
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
        .route("/api/agent/pair", post(agents::pair))
        .route_layer(from_fn_with_state(state.clone(), web::rate_limit));

    Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/config", get(auth::config))
        .route("/api/auth/google/start", get(google::start))
        .route("/api/auth/google/callback", get(google::callback))
        .route("/api/auth/google", axum::routing::delete(google::unlink))
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
        .route("/api/assets/export", get(assets::export))
        .route("/api/assets/{id}/archive", post(assets::archive))
        .route("/api/assets/{id}/restore", post(assets::restore))
        .route("/api/assets/{id}", get(assets::get).patch(assets::update))
        .route("/api/assets/{id}/events", get(assets::events))
        .route("/api/assets/{id}/assign", post(assets::assign))
        .route("/api/assets/{id}/return", post(assets::return_asset))
        .route("/api/categories", get(catalog::list_categories).post(catalog::create_category))
        .route("/api/categories/{id}", patch(catalog::update_category).delete(catalog::delete_category))
        .route("/api/tags", get(catalog::list_tags))
        .route("/api/tags/{id}", patch(catalog::update_tag).delete(catalog::delete_tag))
        .route("/api/custom-fields", get(catalog::list_fields).post(catalog::create_field))
        .route("/api/custom-fields/{id}", patch(catalog::update_field).delete(catalog::delete_field))
        .route(
            "/api/assets/{id}/attachments",
            get(attachments::list)
                .post(attachments::upload)
                .layer(axum::extract::DefaultBodyLimit::max(state.config.max_upload_bytes + 256 * 1024)),
        )
        .route("/api/attachments/{id}", get(attachments::download).delete(attachments::delete))
        .route("/api/assets/{id}/warranties", get(warranties::list).post(warranties::create))
        .route("/api/warranties/{id}", patch(warranties::update).delete(warranties::delete))
        .route("/api/notifications", get(notifications::list))
        .route("/api/notifications/read-all", post(notifications::read_all))
        .route("/api/notifications/{id}/read", post(notifications::read))
        .route("/api/notifications/{id}", axum::routing::delete(notifications::delete))
        .route("/api/notification-settings", get(notifications::get_settings).patch(notifications::update_settings))
        .route("/api/notification-settings/test", post(notifications::send_test))
        .route("/api/push/subscriptions", post(notifications::subscribe_push).delete(notifications::unsubscribe_push))
        .route("/api/reminders", get(user_reminders::list).post(user_reminders::create))
        .route(
            "/api/reminders/{id}",
            get(user_reminders::get).patch(user_reminders::update).delete(user_reminders::delete),
        )
        .route("/api/reminders/{id}/snooze", post(user_reminders::snooze))
        .route("/api/reminders/{id}/complete", post(user_reminders::complete))
        .route("/api/deadlines", get(notifications::deadlines))
        .route("/api/insights", get(notifications::insights))
        .route("/api/plan", get(plans::get))
        .route("/api/billing/checkout", post(plans::checkout))
        .route("/api/agents", get(agents::list_agents))
        .route("/api/agents/pairing-codes", post(agents::create_pairing_code))
        .route("/api/agents/{id}", patch(agents::update_agent).delete(agents::revoke_agent))
        .route("/api/discovered-devices", get(agents::list_devices))
        .route("/api/discovered-devices/{id}/add", post(agents::add_device))
        .route("/api/discovered-devices/{id}/link", post(agents::link_device))
        .route("/api/discovered-devices/{id}/{action}", post(agents::set_device_state))
        .route("/api/agent/status", get(agents::status))
        .route("/api/agent/report", post(agents::report))
        .route("/api/agent/disconnect", post(agents::disconnect))
        .route("/api/inventories", get(inventories::list).post(inventories::create))
        .route("/api/inventories/{id}", get(inventories::get).patch(inventories::update))
        .route("/api/inventories/{id}/archive", post(inventories::archive))
        .route("/api/inventories/{id}/restore", post(inventories::restore))
        .route("/api/transfers", get(transfers::list).post(transfers::create))
        .route("/api/transfers/{id}", get(transfers::get))
        .route("/api/transfers/{id}/approve", post(transfers::approve))
        .route("/api/transfers/{id}/reject", post(transfers::reject))
        .route("/api/transfers/{id}/cancel", post(transfers::cancel))
        .route("/api/loans", get(loans::list).post(loans::create))
        .route("/api/loans/export", get(loans::export))
        .route("/api/loans/{id}", get(loans::get))
        .route("/api/loans/{id}/deploy", post(loans::deploy))
        .route("/api/loans/{id}/extend", post(loans::extend))
        .route("/api/loans/{id}/return", post(loans::return_loan))
        .route("/api/loans/{id}/inspect", post(loans::inspect))
        .route("/api/loans/{id}/cancel", post(loans::cancel))
        .route("/api/loans/{id}/lost", post(loans::lost))
        .route("/api/access-requests", get(access_requests::list).post(access_requests::create))
        .route("/api/access-requests/{id}/approve", post(access_requests::approve))
        .route("/api/access-requests/{id}/reject", post(access_requests::reject))
        .route("/api/access-requests/{id}/cancel", post(access_requests::cancel))
        .route("/api/people", get(people::list).post(people::create))
        .route("/api/people/{id}", patch(people::update))
        .route("/api/audits", get(audits::list).post(audits::start))
        .route("/api/audits/{id}", get(audits::get))
        .route("/api/audits/{id}/scan", post(audits::scan))
        .route("/api/audits/{id}/items/{asset_id}", patch(audits::mark_item))
        .route("/api/audits/{id}/close", post(audits::close))
        .route("/api/audits/{id}/begin", post(audits::begin))
        .route("/api/audits/{id}/submit", post(audits::submit))
        .route("/api/audits/{id}/approve", post(audits::approve))
        .route("/api/audits/{id}/reject", post(audits::reject))
        .route("/api/audits/{id}/reopen", post(audits::reopen))
        .route("/api/audits/{id}/export", get(audits::export))
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
