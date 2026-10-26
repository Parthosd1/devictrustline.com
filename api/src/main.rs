//! DeviceTrustline API server.
//!
//! `devicetrustline` (or `devicetrustline serve`) runs migrations, the audit scheduler and the HTTP
//! server. Other commands: `migrate`, `create-admin`, `seed`, `run-scheduled-audits` and
//! `healthcheck` (for container health checks; exits non-zero unless /api/health answers 200).

mod auth;
mod config;
mod db;
mod error;
mod mail;
mod routes;
mod scheduler;
mod tokens;
mod validate;
mod web;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::middleware::from_fn_with_state;
use axum::Router;
use sqlx::PgPool;

use crate::config::Config;
use crate::db::Q;
use crate::web::RateLimiter;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub auth_limiter: Arc<RateLimiter>,
}

pub fn app(state: AppState) -> Router {
    let mut router = routes::api(&state).layer(from_fn_with_state(state.clone(), web::check_origin));
    if let Some(dir) = &state.config.static_dir {
        router = router.merge(web::web_app(dir));
    }
    router
        .layer(from_fn_with_state(state.clone(), web::security_headers))
        .layer(from_fn_with_state(state.clone(), web::request_log))
        .with_state(state)
}

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        std::process::exit(if healthcheck().await { 0 } else { 1 });
    }
    let config = Config::from_env().unwrap_or_else(|e| fail(e));
    let pool = db::connect(&config).await.unwrap_or_else(|e| fail(e));
    let command = std::env::args().nth(1).unwrap_or_else(|| "serve".into());
    match command.as_str() {
        "serve" => serve(config, pool).await,
        "migrate" => db::migrate(&pool, true).await.unwrap_or_else(|e| fail(format!("{e:?}"))),
        "create-admin" => create_admin(&pool).await,
        "seed" => seed(&pool).await,
        "run-scheduled-audits" => {
            let started =
                scheduler::run_scheduled_audits(&pool, false).await.unwrap_or_else(|e| fail(format!("{e:?}")));
            println!("{}", serde_json::Value::Array(started));
        }
        "run-jobs" => {
            let done = scheduler::run_jobs(&pool, &config).await.unwrap_or_else(|e| fail(format!("{e:?}")));
            println!("{done}");
        }
        other => fail(format!(
            "Unknown command {other}. Use serve, migrate, create-admin, seed, run-scheduled-audits, run-jobs or healthcheck."
        )),
    }
}

async fn serve(config: Config, pool: PgPool) {
    db::migrate(&pool, true).await.unwrap_or_else(|e| fail(format!("{e:?}")));
    if std::env::var("DISABLE_SCHEDULER").as_deref() != Ok("true") {
        scheduler::start(pool.clone(), config.clone());
    }
    let port = config.port;
    let state = AppState {
        auth_limiter: Arc::new(RateLimiter::new(config.auth_rate_limit, Duration::from_secs(15 * 60))),
        config: Arc::new(config),
        pool: pool.clone(),
    };
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await.unwrap_or_else(|e| fail(e));
    println!("DeviceTrustline API listening on :{port}");
    axum::serve(listener, app(state).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown())
        .await
        .unwrap_or_else(|e| fail(e));
    pool.close().await;
}

async fn healthcheck() -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let port = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(4000u16);
    let check = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        let request = format!("GET /api/health HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut head = [0u8; 12];
        stream.read_exact(&mut head).await.ok()?;
        Some(head.starts_with(b"HTTP/1.1 200"))
    };
    tokio::time::timeout(Duration::from_secs(4), check).await.ok().flatten().unwrap_or(false)
}

async fn shutdown() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.ok() };
    #[cfg(unix)]
    let term = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap().recv().await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

/// Creates an organization and its first administrator, with no sample data.
/// Usage: ORG_NAME='Acme' ADMIN_EMAIL=it@acme.com ADMIN_NAME='Pat Lee' ADMIN_PASSWORD='a long password' devicetrustline create-admin
async fn create_admin(pool: &PgPool) {
    let var = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let (Some(org), Some(email), Some(password)) =
        (var("ORG_NAME"), var("ADMIN_EMAIL"), std::env::var("ADMIN_PASSWORD").ok())
    else {
        fail("Set ORG_NAME, ADMIN_EMAIL and ADMIN_PASSWORD (and optionally ADMIN_NAME).");
    };
    let name = var("ADMIN_NAME").unwrap_or_else(|| "Administrator".into());
    if password.chars().count() < 12 {
        fail("Password must be at least 12 characters");
    }
    db::migrate(pool, true).await.unwrap_or_else(|e| fail(format!("{e:?}")));
    let hash = auth::hash_password(password).await.unwrap_or_else(|e| fail(format!("{e:?}")));
    let result = async {
        let mut tx = pool.begin().await?;
        // The administrator set up from the server is trusted with their own address.
        let user_id: uuid::Uuid = Q::new(
            "INSERT INTO users (email, name, password_hash, email_verified_at) VALUES ($1::citext, $2, $3, now()) RETURNING id",
        )
        .bind(&email)
        .bind(&name)
        .bind(hash)
        .scalar(&mut tx)
        .await?;
        routes::auth::create_workspace(&mut tx, user_id, &org, "business").await?;
        tx.commit().await?;
        Ok::<_, error::AppError>(())
    }
    .await;
    match result {
        Ok(()) => println!("Created {org} with administrator {email}. Sign in and add your team under Users."),
        Err(e) if e.pg_code().as_deref() == Some("23505") => fail(format!("{email} already has an account.")),
        Err(e) => fail(format!("{e:?}")),
    }
}

/// Creates a demo organization with the same sample inventory the frontend MVP ships with.
/// Usage: SEED_ADMIN_EMAIL=you@example.com SEED_ADMIN_PASSWORD='a long password' devicetrustline seed
async fn seed(pool: &PgPool) {
    const SAMPLE: &[[&str; 6]] = &[
        ["Dell Latitude 5440", "Laptop", "DL5440-9921", "Building A", "IT Storage", "Available"],
        ["Zebra DS3678", "Scanner", "ZB-3678-112", "Building A", "Station 214-02", "Deployed"],
        ["HP EliteDesk 800", "Desktop", "HP800-4218", "Building A", "Station 214-02", "Deployed"],
        ["Dell P2422H", "Monitor", "DP24-9922", "Building A", "Station 214-02", "Deployed"],
        ["Honeywell 1950g", "Scanner", "HW-1950-555", "Building B", "Station 4151", "Maintenance"],
        ["Lenovo ThinkPad T14", "Laptop", "LT14-2180", "Building B", "Loaner Storage", "Available"],
        ["Logitech C920", "Accessory", "LC920-001", "Building A", "Station 214-02", "Deployed"],
        ["Zebra TC52", "Scanner", "TC52-8290", "Building B", "Station 4151", "Deployed"],
    ];
    let email = std::env::var("SEED_ADMIN_EMAIL").unwrap_or_default().to_lowercase();
    let password = std::env::var("SEED_ADMIN_PASSWORD").unwrap_or_default();
    if email.is_empty() || password.chars().count() < 12 {
        fail("Set SEED_ADMIN_EMAIL and SEED_ADMIN_PASSWORD (12+ characters).");
    }
    db::migrate(pool, true).await.unwrap_or_else(|e| fail(format!("{e:?}")));
    let hash = auth::hash_password(password).await.unwrap_or_else(|e| fail(format!("{e:?}")));
    let result = async {
        let mut tx = pool.begin().await?;
        let admin: uuid::Uuid = Q::new(
            "INSERT INTO users (email, name, password_hash, email_verified_at) VALUES ($1::citext, 'Demo Admin', $2, now()) RETURNING id",
        )
        .bind(&email)
        .bind(hash)
        .scalar(&mut tx)
        .await?;
        let org = routes::auth::create_workspace(&mut tx, admin, "Demo Workspace", "business").await?;
        let mut buildings = std::collections::HashMap::new();
        let mut locations = std::collections::HashMap::new();
        for [name, kind, serial, building, location, status] in SAMPLE {
            if !buildings.contains_key(building) {
                let id: uuid::Uuid = Q::new("INSERT INTO buildings (org_id, name) VALUES ($1, $2) RETURNING id")
                    .bind(org)
                    .bind(*building)
                    .scalar(&mut tx)
                    .await?;
                buildings.insert(*building, id);
            }
            let key = format!("{building}/{location}");
            if !locations.contains_key(&key) {
                let id: uuid::Uuid = Q::new("INSERT INTO locations (org_id, building_id, name) VALUES ($1, $2, $3) RETURNING id")
                    .bind(org)
                    .bind(buildings[building])
                    .bind(*location)
                    .scalar(&mut tx)
                    .await?;
                locations.insert(key.clone(), id);
            }
            let n: i32 = Q::new(
                "UPDATE organizations SET next_asset_number = next_asset_number + 1 WHERE id = $1 RETURNING next_asset_number - 1",
            )
            .bind(org)
            .scalar(&mut tx)
            .await?;
            let id: uuid::Uuid = Q::new(
                "INSERT INTO assets (org_id, asset_tag, name, type, serial, status, building_id, location_id, created_by)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
            )
            .bind(org)
            .bind(format!("DT-{n}"))
            .bind(*name)
            .bind(*kind)
            .bind(*serial)
            .bind(*status)
            .bind(buildings[building])
            .bind(locations[&key])
            .bind(admin)
            .scalar(&mut tx)
            .await?;
            let changes = serde_json::json!({ "source": { "from": null, "to": "seed" } });
            routes::assets::record_event(&mut tx, org, id, admin, "created", changes).await?;
        }
        tx.commit().await?;
        Ok::<_, error::AppError>(())
    }
    .await;
    match result {
        Ok(()) => println!("Seeded demo workspace. Sign in as {email}."),
        Err(e) => fail(format!("{e:?}")),
    }
}
