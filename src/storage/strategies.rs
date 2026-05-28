//! Запись StrategySnapshot в `strategies` + `strategy_versions`.
//!
//! Семантика:
//!   * UPSERT в `strategies` по (server_id, moonbot_id).
//!   * Новая строка в `strategy_versions` — только если изменился
//!     `LastEditDate` (то что Moonbot обновляет при реальной правке
//!     параметров, не на каждом state-update).
//!
//! Public-схема не хранит is_mutable / risk_stop_count — это поля Cloner/Risk.

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use postgres::Client;
use rust_decimal::Decimal;
use serde_json::{Map, Value};

use moonproto::commands::strategy_serializer::{FieldValue, StrategyActiveMode, StrategySnapshot};

pub fn upsert_snapshot(
    client: &mut Client,
    server_id: i32,
    snap: &StrategySnapshot,
) -> anyhow::Result<i32> {
    let name = extract_string(snap, "StrategyName").unwrap_or_default();
    let signal_type = extract_string(snap, "SignalType");
    // `checked` — raw UI checkbox state. `is_active` — Delphi-equivalent
    // вычисление для нашего режима (UsingMoonProto). См. types.rs:355.
    let checked = snap.checked;
    let is_active = snap.is_active(StrategyActiveMode::UsingMoonProto);

    let moonbot_id = Decimal::from(snap.strategy_id);

    let row = client.query_one(
        "INSERT INTO strategies (server_id, moonbot_id, name, signal_type, checked, is_active, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, NOW()) \
         ON CONFLICT (server_id, moonbot_id) DO UPDATE SET \
            name        = EXCLUDED.name, \
            signal_type = COALESCE(EXCLUDED.signal_type, strategies.signal_type), \
            checked     = EXCLUDED.checked, \
            is_active   = EXCLUDED.is_active, \
            updated_at  = NOW() \
         RETURNING id",
        &[&server_id, &moonbot_id, &name, &signal_type, &checked, &is_active],
    )?;
    let strategy_id: i32 = row.get(0);

    // Версионирование: новая строка только если LastEditDate изменился.
    let led_now = current_led_from_snap(snap);
    let raw_data = fields_to_json(snap);

    let prev_led: Option<DateTime<Utc>> = client
        .query_opt(
            "SELECT (raw_data->>'LastEditDate') AS led FROM strategy_versions \
             WHERE strategy_id = $1 \
             ORDER BY version_date DESC LIMIT 1",
            &[&strategy_id],
        )?
        .and_then(|r| r.get::<_, Option<String>>(0))
        .and_then(|s| parse_led_utc(&s));

    let needs_new_version = match (led_now, prev_led) {
        (Some(now), Some(prev)) => now != prev,
        (Some(_), None) => true,
        (None, _) => false,
    };

    if needs_new_version {
        client.execute(
            "INSERT INTO strategy_versions (strategy_id, version_date, raw_data) \
             VALUES ($1, NOW(), $2) \
             ON CONFLICT (strategy_id, version_date) DO NOTHING",
            &[&strategy_id, &Value::Object(raw_data)],
        )?;
    }
    Ok(strategy_id)
}

// ── Helpers ──────────────────────────────────────────────────────

fn extract_string(snap: &StrategySnapshot, name: &str) -> Option<String> {
    snap.fields.get(name).and_then(|v| match v {
        FieldValue::String(s) => Some(s.clone()),
        _ => None,
    })
}

fn current_led_from_snap(snap: &StrategySnapshot) -> Option<DateTime<Utc>> {
    let s = match snap.fields.get("LastEditDate")? {
        FieldValue::String(s) => s.trim().to_string(),
        _ => return None,
    };
    parse_led_utc(&s)
}

fn parse_led_utc(s: &str) -> Option<DateTime<Utc>> {
    if s.is_empty() {
        return None;
    }
    for fmt in &["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(Utc.from_utc_datetime(&naive));
        }
    }
    None
}

fn fields_to_json(snap: &StrategySnapshot) -> Map<String, Value> {
    let mut out = Map::new();
    for (name, value) in snap.fields.iter() {
        out.insert(name.to_string(), field_to_json(value));
    }
    out.insert("__strategy_ver".to_string(), Value::from(snap.strategy_ver));
    out.insert("__kind".to_string(), Value::from(snap.kind));
    out.insert("__path".to_string(), Value::from(snap.path.clone()));
    out
}

fn field_to_json(v: &FieldValue) -> Value {
    match v {
        FieldValue::Bool(b)   => Value::from(*b),
        FieldValue::Int32(x)  => Value::from(*x),
        FieldValue::Int64(x)  => Value::from(*x),
        FieldValue::Double(x) => Value::from(*x),
        FieldValue::Single(x) => Value::from(*x as f64),
        FieldValue::String(s) => Value::from(s.clone()),
        FieldValue::Byte(x)   => Value::from(*x),
        FieldValue::Word(x)   => Value::from(*x),
        FieldValue::UInt32(x) => Value::from(*x),
        FieldValue::UInt64(x) => Value::from(*x),
    }
}
