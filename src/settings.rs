//! Чтение `app_settings` с TTL-кэшем.
//!
//! Каждое значение читается из БД не чаще раза в [`CACHE_TTL`]; между
//! обращениями отдаётся cached копия. Это позволяет менять параметры в
//! рантайме (UPDATE app_settings) без рестарта процесса и при этом не
//! ходить в БД на каждый event.
//!
//! Использование:
//! ```
//! let timeout_secs = settings::get_int("connect_timeout_secs", 15);
//! let poll_ms = settings::get_int("event_poll_ms", 500);
//! ```

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use postgres::Client;

const CACHE_TTL: Duration = Duration::from_secs(60);

struct Cache {
    values: HashMap<String, String>,
    loaded_at: Instant,
    db_url: Option<String>,
}

impl Cache {
    fn empty() -> Self {
        Self {
            values: HashMap::new(),
            loaded_at: Instant::now() - Duration::from_secs(3600),
            db_url: None,
        }
    }
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| Mutex::new(Cache::empty()))
}

/// Должно вызываться один раз при старте — после этого settings::get_*
/// смогут лениво открывать соединение для refresh-а.
pub fn init(db_url: impl Into<String>) {
    let mut c = cache().lock().expect("settings cache lock");
    c.db_url = Some(db_url.into());
}

fn refresh_if_due(c: &mut Cache) {
    if c.loaded_at.elapsed() < CACHE_TTL && !c.values.is_empty() {
        return;
    }
    let Some(url) = c.db_url.clone() else { return };

    // Открываем короткое соединение только для refresh. Альтернатива —
    // r2d2 pool, но для двух-трёх десятков параметров overkill.
    let mut client = match Client::connect(&url, postgres::NoTls) {
        Ok(cl) => cl,
        Err(e) => {
            tracing::warn!("settings refresh: DB connect failed: {e}");
            c.loaded_at = Instant::now();
            return;
        }
    };
    match client.query("SELECT key, value FROM app_settings", &[]) {
        Ok(rows) => {
            c.values.clear();
            for row in rows {
                let k: String = row.get(0);
                let v: String = row.get(1);
                c.values.insert(k, v);
            }
            c.loaded_at = Instant::now();
        }
        Err(e) => {
            tracing::warn!("settings refresh: query failed: {e}");
            c.loaded_at = Instant::now();
        }
    }
}

pub fn get(key: &str) -> Option<String> {
    let mut c = cache().lock().expect("settings cache lock");
    refresh_if_due(&mut c);
    c.values.get(key).cloned()
}

pub fn get_str(key: &str, default: &str) -> String {
    get(key).unwrap_or_else(|| default.to_string())
}

pub fn get_int(key: &str, default: i64) -> i64 {
    get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

pub fn get_bool(key: &str, default: bool) -> bool {
    get(key)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "1" | "on" | "enabled"))
        .unwrap_or(default)
}

pub fn poll_interval() -> Duration {
    Duration::from_millis(get_int("event_poll_ms", 500).max(50) as u64)
}

pub fn connect_timeout() -> Duration {
    Duration::from_secs(get_int("connect_timeout_secs", 15).max(1) as u64)
}
