//! mb_ingester — entry point.
//!
//! 1. Загружаем config.toml (только [db].url)
//! 2. Подключаемся к БД, применяем миграцию (идемпотентно)
//! 3. Инициализируем settings-кэш (TTL 60s, читает app_settings)
//! 4. Загружаем servers (фильтр: token есть, какой-то listener_* включён)
//! 5. Для каждого сервера запускаем thread с MoonProto-сессией.

mod config;
mod db;
mod session;
mod settings;
mod storage;

use std::thread;

use anyhow::Context;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::config::AppConfig;

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    init_logging();

    let cfg = AppConfig::load().context("config load failed")?;
    tracing::info!("loaded config: db={}", sanitize_url(&cfg.db.url));

    // Применить миграцию + ping
    let mut sql = db::connect_and_migrate(&cfg.db.url)
        .context("DB connect_and_migrate failed")?;
    let _ = sql.query_one("SELECT 1", &[])?;
    tracing::info!("DB ping ok");

    settings::init(cfg.db.url.clone());

    let servers = db::load_servers(&mut sql)?;
    tracing::info!("loaded {} ingestable servers from DB", servers.len());
    for s in &servers {
        tracing::info!(
            "  · #{} {} → {}:{}  modules={{strategies={}, orders={}}}",
            s.id, s.name, s.ip, s.port,
            s.modules.listener_strategies, s.modules.listener_orders
        );
    }
    if servers.is_empty() {
        anyhow::bail!(
            "No servers found. INSERT into servers with token + modules='{{\"listener_orders\":true,\"listener_strategies\":true}}'::jsonb"
        );
    }
    drop(sql); // главный коннект больше не нужен

    let mut handles = Vec::new();
    for server in servers {
        let db_url = cfg.db.url.clone();
        let name = server.name.clone();
        let h = thread::Builder::new()
            .name(format!("mp-{name}"))
            .spawn(move || {
                if let Err(e) = session::run_session(&server, &db_url) {
                    tracing::error!("[{name}] session terminated: {e:#}");
                }
            })?;
        handles.push(h);
    }
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,moonproto=info,mb_ingester=debug"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true).with_thread_names(true))
        .init();
}

fn sanitize_url(url: &str) -> String {
    if let Some(at) = url.find('@') {
        if let Some(scheme_end) = url.find("://") {
            let prefix = &url[..scheme_end + 3];
            let host_part = &url[at..];
            return format!("{prefix}***{host_part}");
        }
    }
    url.to_string()
}
