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
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::config::AppConfig;

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let _log_guard = init_logging();

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
            "  · #{} {}  modules={{strategies={}, orders={}}}",
            s.id, s.name,
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

/// Инициализация tracing:
///   * файл `logs/mb_ingester.YYYY-MM-DD` — дневная ротация, полный фильтр
///   * stderr — только WARN/ERROR (терминал тихий, видны только проблемы)
///
/// Уровень логирования настраивается через `RUST_LOG` (env-filter синтаксис).
/// Возвращаемый guard нужно держать живым на всё время работы программы,
/// иначе buffered-запись в файл потеряется на shutdown.
fn init_logging() -> WorkerGuard {
    let log_dir = std::env::var("MB_INGESTER_LOG_DIR").unwrap_or_else(|_| "logs".into());
    std::fs::create_dir_all(&log_dir).expect("create log dir");

    let file_appender = tracing_appender::rolling::daily(&log_dir, "mb_ingester.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    // moonproto::crypted=error глушит спам "replay/duplicate detected" —
    // нормальный UDP-шум, протокол сам дедуплицирует. Если нужно отлаживать
    // транспорт — RUST_LOG=info,moonproto::crypted=debug.
    let default_filter = "info,moonproto=info,mb_ingester=debug,moonproto::crypted=error";
    let file_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_filter));
    let stderr_filter = EnvFilter::new("warn,moonproto::crypted=error");

    let file_layer = fmt::layer()
        .with_target(true)
        .with_thread_names(true)
        .with_ansi(false)
        .with_writer(file_writer)
        .with_filter(file_filter);

    let stderr_layer = fmt::layer()
        .with_target(true)
        .with_thread_names(true)
        .with_filter(stderr_filter);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .init();

    guard
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
