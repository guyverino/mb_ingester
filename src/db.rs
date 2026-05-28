//! PostgreSQL — connect, миграции, загрузка серверов.

use postgres::{Client, NoTls};

use crate::config::{DbServer, ServerModules};

/// Подключение + автоприменение миграций в порядке `migrations/NNN_*.sql`.
/// Все скрипты идемпотентны (CREATE … IF NOT EXISTS + sentinel-проверки).
pub fn connect_and_migrate(url: &str) -> anyhow::Result<Client> {
    let mut client = Client::connect(url, NoTls)
        .map_err(|e| anyhow::anyhow!("DB connect failed: {e}"))?;

    let migrations: &[(&str, &str)] = &[
        ("001_initial", include_str!("../migrations/001_initial.sql")),
        ("002_orders_wide", include_str!("../migrations/002_orders_wide.sql")),
        ("003_strategies_active", include_str!("../migrations/003_strategies_active.sql")),
        ("004_strategies_wide", include_str!("../migrations/004_strategies_wide.sql")),
        ("005_orders_strategy_version_link", include_str!("../migrations/005_orders_strategy_version_link.sql")),
        ("006_parameters", include_str!("../migrations/006_parameters.sql")),
    ];

    for (name, sql) in migrations {
        client
            .batch_execute(sql)
            .map_err(|e| anyhow::anyhow!("migration {name} failed: {e}"))?;
        tracing::debug!("applied migration {name}");
    }
    tracing::info!("DB migrations applied ({} files)", migrations.len());
    Ok(client)
}

pub fn connect(url: &str) -> anyhow::Result<Client> {
    Client::connect(url, NoTls).map_err(|e| anyhow::anyhow!("DB connect failed: {e}"))
}

/// Загружает все серверы, у которых:
///   * заполнен `token`
///   * хотя бы один listener-модуль включён в `modules` JSONB
pub fn load_servers(client: &mut Client) -> anyhow::Result<Vec<DbServer>> {
    let rows = client.query(
        "SELECT id, name, ip, port, token, modules \
         FROM servers \
         WHERE token IS NOT NULL AND length(token) > 0 \
         ORDER BY id",
        &[],
    )?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let modules_json: serde_json::Value = row.get::<_, serde_json::Value>(5);
        let modules = ServerModules::from_json(&modules_json);
        if !modules.any_enabled() {
            tracing::info!(
                "skip server #{} {} — no listener_* modules enabled",
                row.get::<_, i32>(0),
                row.get::<_, String>(1)
            );
            continue;
        }
        out.push(DbServer {
            id: row.get(0),
            name: row.get(1),
            ip: row.get(2),
            port: row.get(3),
            token: row.get(4),
            modules,
        });
    }
    Ok(out)
}
