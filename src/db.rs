//! PostgreSQL — connect, миграции, загрузка серверов.

use postgres::{Client, NoTls};

use crate::config::{DbServer, ServerModules};

/// Подключение + автоприменение `migrations/001_initial.sql`.
/// Скрипт идемпотентный (CREATE … IF NOT EXISTS + ON CONFLICT DO NOTHING).
pub fn connect_and_migrate(url: &str) -> anyhow::Result<Client> {
    let mut client = Client::connect(url, NoTls)
        .map_err(|e| anyhow::anyhow!("DB connect failed: {e}"))?;
    let sql = include_str!("../migrations/001_initial.sql");
    client
        .batch_execute(sql)
        .map_err(|e| anyhow::anyhow!("migration failed: {e}"))?;
    tracing::info!("DB migrations applied");
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
