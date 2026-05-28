//! Запись Order'ов в `orders` (public-схема).
//!
//! Поля Public-схемы — только то, что реально есть в `moonproto::state::Order`.
//! Никаких enriched-метрик (deltas, pump1h, dvol …) — это уже задача
//! analytics-модулей поверх ingester.

use chrono::{DateTime, TimeZone, Utc};
use moonproto::state::Order;
use postgres::Client;
use rust_decimal::Decimal;

pub fn upsert(client: &mut Client, server_id: i32, order: &Order) -> anyhow::Result<()> {
    let uid = order.uid as i64;
    let strat_id = Decimal::from(order.strat_id);
    let status_code = order.status.0 as i32;

    let buy_date     = delphi_to_utc(order.buy_order.open_time);
    let sell_set     = delphi_to_utc(order.sell_order.open_time);
    let close_date   = delphi_to_utc(
        if order.sell_order.close_time != 0.0 {
            order.sell_order.close_time
        } else {
            order.buy_order.close_time
        },
    );

    let spent_btc  = order.buy_order.spent_btc;
    let gained_btc = order.sell_order.total_btc;
    let profit_btc = if gained_btc > 0.0 { gained_btc - spent_btc } else { 0.0 };

    client.execute(
        "INSERT INTO orders (
            server_id, id, coin, strategy_id, emulator, is_short, status,
            quantity, buy_price, sell_price, spent_btc, gained_btc, profit_btc,
            sell_reason, buy_date, sell_set_date, close_date, updated_at
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7,
            $8, $9, $10, $11, $12, $13,
            $14, $15, $16, $17, NOW()
         )
         ON CONFLICT (server_id, id) DO UPDATE SET
            coin         = COALESCE(EXCLUDED.coin, orders.coin),
            strategy_id  = EXCLUDED.strategy_id,
            emulator     = EXCLUDED.emulator,
            is_short     = EXCLUDED.is_short,
            status       = EXCLUDED.status,
            quantity     = EXCLUDED.quantity,
            buy_price    = EXCLUDED.buy_price,
            sell_price   = EXCLUDED.sell_price,
            spent_btc    = EXCLUDED.spent_btc,
            gained_btc   = EXCLUDED.gained_btc,
            profit_btc   = EXCLUDED.profit_btc,
            sell_reason  = COALESCE(EXCLUDED.sell_reason, orders.sell_reason),
            buy_date     = COALESCE(EXCLUDED.buy_date, orders.buy_date),
            sell_set_date= COALESCE(EXCLUDED.sell_set_date, orders.sell_set_date),
            close_date   = COALESCE(EXCLUDED.close_date, orders.close_date),
            updated_at   = NOW()",
        &[
            &server_id, &uid, &order.market_name, &strat_id,
            &order.emulator_mode, &order.is_short, &status_code,
            &order.buy_order.quantity, &order.buy_price, &order.sell_price,
            &spent_btc, &gained_btc, &profit_btc,
            &sell_reason_text(order.sell_reason_code),
            &buy_date, &sell_set, &close_date,
        ],
    )?;
    Ok(())
}

pub fn delete(client: &mut Client, server_id: i32, uid: u64) -> anyhow::Result<()> {
    client.execute(
        "DELETE FROM orders WHERE server_id = $1 AND id = $2",
        &[&server_id, &(uid as i64)],
    )?;
    Ok(())
}

/// Удалить все ордера сервера, которых нет в текущем snapshot.
/// Включать через app_setting `orders_sync_on_snapshot`.
pub fn sync_snapshot(
    client: &mut Client,
    server_id: i32,
    active_uids: impl IntoIterator<Item = u64>,
) -> anyhow::Result<usize> {
    let active: Vec<i64> = active_uids.into_iter().map(|u| u as i64).collect();
    if active.is_empty() {
        let n = client.execute("DELETE FROM orders WHERE server_id = $1", &[&server_id])?;
        return Ok(n as usize);
    }
    let n = client.execute(
        "DELETE FROM orders WHERE server_id = $1 AND id <> ALL($2)",
        &[&server_id, &active],
    )?;
    Ok(n as usize)
}

// ── Delphi-time → UTC ──────────────────────────────────────────

const DELPHI_UNIX_EPOCH: f64 = 25569.0; // дни между 1899-12-30 и 1970-01-01

fn delphi_to_utc(delphi: f64) -> Option<DateTime<Utc>> {
    if delphi == 0.0 || !delphi.is_finite() {
        return None;
    }
    let unix_sec_f = (delphi - DELPHI_UNIX_EPOCH) * 86400.0;
    let secs = unix_sec_f.trunc() as i64;
    let nsecs = ((unix_sec_f.fract() * 1e9) as i64).max(0) as u32;
    Utc.timestamp_opt(secs, nsecs).single()
}

/// Минимальный маппинг кодов закрытия. Полный список — в moonproto::state::SellReason.
fn sell_reason_text(code: u8) -> Option<String> {
    match code {
        0 => None,
        1 => Some("Sell Price".to_string()),
        2 => Some("Auto Price Down".to_string()),
        3 => Some("Sell Level".to_string()),
        4 => Some("Stop Loss".to_string()),
        5 => Some("Trailing".to_string()),
        _ => Some(format!("code={code}")),
    }
}
