//! Минимальный конфиг — только подключение к БД.
//!
//! Все остальные параметры (timeouts, polling intervals и т.д.) живут
//! в `app_settings` БД и читаются через [`crate::settings`].
//!
//! Список серверов — в таблице `servers` БД.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub db: DbConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DbConfig {
    pub url: String,
}

impl AppConfig {
    pub fn load() -> anyhow::Result<Self> {
        let path = std::env::var("MB_INGESTER_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("config.toml"));
        Self::load_from(&path)
    }

    pub fn load_from(path: &std::path::Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
        let cfg: AppConfig = toml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("config parse error in {}: {e}", path.display()))?;
        Ok(cfg)
    }
}

/// Сервер, прочитанный из таблицы `servers`. Уже отфильтрован: token не пуст,
/// enabled-модули содержат хотя бы один `listener_*`.
#[derive(Debug, Clone)]
pub struct DbServer {
    pub id: i32,
    pub name: String,
    pub ip: String,
    pub port: i32,
    pub token: String,
    /// Включённые модули, ключи как в `modules.name`.
    pub modules: ServerModules,
}

#[derive(Debug, Clone, Default)]
pub struct ServerModules {
    pub listener_strategies: bool,
    pub listener_orders: bool,
}

impl ServerModules {
    pub fn from_json(v: &serde_json::Value) -> Self {
        let bool_at = |k: &str| -> bool {
            v.get(k).and_then(|x| x.as_bool()).unwrap_or(false)
        };
        Self {
            listener_strategies: bool_at("listener_strategies"),
            listener_orders: bool_at("listener_orders"),
        }
    }

    pub fn any_enabled(&self) -> bool {
        self.listener_strategies || self.listener_orders
    }
}
