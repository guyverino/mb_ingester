//! Запись `Order` в таблицу `orders` — журнал реальных сделок.
//!
//! Содержит полную копию структуры `Order` из moonproto:
//!   * top-level поля (status, prices, vstop, panic, corridor, …)
//!   * buy_order / sell_order (OrderCompact, по 23 поля)
//!   * stops (StopSettings, 11 полей)
//!
//! UPSERT по PRIMARY KEY (server_id, id). При повторном вызове с тем же uid
//! строка обновляется (например, BuyDone → SellSet → SellDone).

use chrono::{DateTime, TimeZone, Utc};
use moonproto::state::Order;
use postgres::Client;
use rust_decimal::Decimal;

pub fn upsert(client: &mut Client, server_id: i32, order: &Order) -> anyhow::Result<()> {
    // Привязываем ордер к текущей последней версии стратегии. Если стратегии
    // нет в БД или версий ещё нет — strategy_version_id остаётся NULL.
    let strategy_version_id = lookup_current_version_id(client, server_id, order.strat_id)?;
    let row = OrderRow::from(server_id, order, strategy_version_id);

    client.execute(SQL, &row.as_params())?;
    Ok(())
}

fn lookup_current_version_id(
    client: &mut Client,
    server_id: i32,
    moonbot_strategy_id: u64,
) -> anyhow::Result<Option<i64>> {
    let strat_id_dec = Decimal::from(moonbot_strategy_id);
    let row = client.query_opt(
        "SELECT sv.id FROM strategy_versions sv
         JOIN strategies s ON sv.strategy_id = s.id
         WHERE s.server_id = $1 AND s.moonbot_id = $2
         ORDER BY sv.version_date DESC LIMIT 1",
        &[&server_id, &strat_id_dec],
    )?;
    Ok(row.map(|r| r.get(0)))
}

/// Все поля плоско в одной структуре. Делает write-side легко читаемым:
/// одна таблица колонок = один список параметров в нужном порядке.
struct OrderRow {
    // Identity (1..7)
    server_id: i32,
    id: Decimal,
    coin: String,
    currency: i16,
    platform: i16,
    strategy_id: Decimal,
    server_db_id: i32,

    // Status & lifecycle (8..18)
    status: i16,
    sell_reason_code: i16,
    sell_reason: Option<String>,
    is_short: bool,
    emulator: bool,
    from_cache: bool,
    job_is_done: bool,
    cancel_request: bool,
    server_forced_remove: bool,
    immune_for_clicks: bool,
    has_local_visual_order: bool,

    // Computed (19..21)
    buy_price: f64,
    sell_price: f64,
    profit_btc: f64,

    // VStop / corridor / panic (22..29)
    vstop_on: bool,
    vstop_fixed: bool,
    vstop_level: f64,
    vstop_vol: f64,
    panic_sell: bool,
    is_moon_shot: bool,
    corridor_price_down: f32,
    corridor_price_up: f32,

    // Pending / replace (30..33)
    pending_buy_cond_price: Option<f64>,
    pending_cancel: bool,
    bulk_replace_buy: bool,
    bulk_replace_sell: bool,

    // Buy OrderCompact (34..56) — 23 поля
    buy_int_id: i64,
    buy_quantity: f64,
    buy_quantity_remaining: f64,
    buy_total_btc: f64,
    buy_spent_btc: f64,
    buy_open_time: Option<DateTime<Utc>>,
    buy_close_time: Option<DateTime<Utc>>,
    buy_actual_price: f64,
    buy_mean_price: f64,
    buy_quantity_base: f64,
    buy_actual_q: f64,
    buy_tmp_btc: f64,
    buy_create_time: Option<DateTime<Utc>>,
    buy_panic_sell_down: f32,
    buy_order_type: i16,
    buy_sub_type: i16,
    buy_stop_flag: i16,
    buy_partial_done: i16,
    buy_leverage: i16,
    buy_is_opened: bool,
    buy_is_closed: bool,
    buy_canceled: bool,
    buy_is_short: bool,

    // Sell OrderCompact (57..79) — те же 23 поля
    sell_int_id: i64,
    sell_quantity: f64,
    sell_quantity_remaining: f64,
    sell_total_btc: f64,
    sell_spent_btc: f64,
    sell_open_time: Option<DateTime<Utc>>,
    sell_close_time: Option<DateTime<Utc>>,
    sell_actual_price: f64,
    sell_mean_price: f64,
    sell_quantity_base: f64,
    sell_actual_q: f64,
    sell_tmp_btc: f64,
    sell_create_time: Option<DateTime<Utc>>,
    sell_panic_sell_down: f32,
    sell_order_type: i16,
    sell_sub_type: i16,
    sell_stop_flag: i16,
    sell_partial_done: i16,
    sell_leverage: i16,
    sell_is_opened: bool,
    sell_is_closed: bool,
    sell_canceled: bool,
    sell_is_short: bool,

    // Stops (80..86) — StopSettings (11 полей)
    stop_loss_on: bool,
    sl_fixed: bool,
    sl_level: f64,
    sl_spread: f64,
    trailing_on: bool,
    trailing_fixed: bool,
    trailing_level: f64,
    ts_spread: f64,
    use_take_profit: bool,
    take_profit: f64,
    take_profit_changed: bool,

    // Link to strategy_versions
    strategy_version_id: Option<i64>,
}

impl OrderRow {
    fn from(server_id: i32, o: &Order, strategy_version_id: Option<i64>) -> Self {
        let profit_btc = if o.sell_order.total_btc > 0.0 {
            o.sell_order.total_btc - o.buy_order.spent_btc
        } else {
            0.0
        };
        Self {
            server_id,
            id: Decimal::from(o.uid),
            coin: o.market_name.clone(),
            currency: o.currency as i16,
            platform: o.platform as i16,
            strategy_id: Decimal::from(o.strat_id),
            server_db_id: o.db_id,

            status: o.status.0 as i16,
            sell_reason_code: o.sell_reason_code as i16,
            sell_reason: sell_reason_text(o.sell_reason_code),
            is_short: o.is_short,
            emulator: o.emulator_mode,
            from_cache: o.from_cache,
            job_is_done: o.job_is_done,
            cancel_request: o.cancel_request,
            server_forced_remove: o.server_forced_remove,
            immune_for_clicks: o.immune_for_clicks,
            has_local_visual_order: o.has_local_visual_order,

            buy_price: o.buy_price,
            sell_price: o.sell_price,
            profit_btc,

            vstop_on: o.vstop_on,
            vstop_fixed: o.vstop_fixed,
            vstop_level: o.vstop_level,
            vstop_vol: o.vstop_vol,
            panic_sell: o.panic_sell,
            is_moon_shot: o.is_moon_shot,
            corridor_price_down: o.corridor_price_down,
            corridor_price_up: o.corridor_price_up,

            pending_buy_cond_price: o.pending_buy_cond_price,
            pending_cancel: o.pending_cancel,
            bulk_replace_buy: o.bulk_replace_buy,
            bulk_replace_sell: o.bulk_replace_sell,

            buy_int_id: o.buy_order.int_id,
            buy_quantity: o.buy_order.quantity,
            buy_quantity_remaining: o.buy_order.quantity_remaining,
            buy_total_btc: o.buy_order.total_btc,
            buy_spent_btc: o.buy_order.spent_btc,
            buy_open_time: delphi_to_utc(o.buy_order.open_time),
            buy_close_time: delphi_to_utc(o.buy_order.close_time),
            buy_actual_price: o.buy_order.actual_price,
            buy_mean_price: o.buy_order.mean_price,
            buy_quantity_base: o.buy_order.quantity_base,
            buy_actual_q: o.buy_order.actual_q,
            buy_tmp_btc: o.buy_order.tmp_btc,
            buy_create_time: delphi_to_utc(o.buy_order.create_time),
            buy_panic_sell_down: o.buy_order.panic_sell_down,
            buy_order_type: o.buy_order.order_type as i16,
            buy_sub_type: o.buy_order.sub_type as i16,
            buy_stop_flag: o.buy_order.stop_flag as i16,
            buy_partial_done: o.buy_order.partial_done as i16,
            buy_leverage: o.buy_order.leverage as i16,
            buy_is_opened: byte_to_bool(o.buy_order.is_opened),
            buy_is_closed: byte_to_bool(o.buy_order.is_closed),
            buy_canceled: byte_to_bool(o.buy_order.canceled),
            buy_is_short: byte_to_bool(o.buy_order.is_short),

            sell_int_id: o.sell_order.int_id,
            sell_quantity: o.sell_order.quantity,
            sell_quantity_remaining: o.sell_order.quantity_remaining,
            sell_total_btc: o.sell_order.total_btc,
            sell_spent_btc: o.sell_order.spent_btc,
            sell_open_time: delphi_to_utc(o.sell_order.open_time),
            sell_close_time: delphi_to_utc(o.sell_order.close_time),
            sell_actual_price: o.sell_order.actual_price,
            sell_mean_price: o.sell_order.mean_price,
            sell_quantity_base: o.sell_order.quantity_base,
            sell_actual_q: o.sell_order.actual_q,
            sell_tmp_btc: o.sell_order.tmp_btc,
            sell_create_time: delphi_to_utc(o.sell_order.create_time),
            sell_panic_sell_down: o.sell_order.panic_sell_down,
            sell_order_type: o.sell_order.order_type as i16,
            sell_sub_type: o.sell_order.sub_type as i16,
            sell_stop_flag: o.sell_order.stop_flag as i16,
            sell_partial_done: o.sell_order.partial_done as i16,
            sell_leverage: o.sell_order.leverage as i16,
            sell_is_opened: byte_to_bool(o.sell_order.is_opened),
            sell_is_closed: byte_to_bool(o.sell_order.is_closed),
            sell_canceled: byte_to_bool(o.sell_order.canceled),
            sell_is_short: byte_to_bool(o.sell_order.is_short),

            stop_loss_on: byte_to_bool(o.stops.stop_loss_on),
            sl_fixed: byte_to_bool(o.stops.sl_fixed),
            sl_level: o.stops.sl_level,
            sl_spread: o.stops.sl_spread,
            trailing_on: byte_to_bool(o.stops.trailing_on),
            trailing_fixed: byte_to_bool(o.stops.trailing_fixed),
            trailing_level: o.stops.trailing_level,
            ts_spread: o.stops.ts_spread,
            use_take_profit: byte_to_bool(o.stops.use_take_profit),
            take_profit: o.stops.take_profit,
            take_profit_changed: byte_to_bool(o.stops.take_profit_changed),
            strategy_version_id,
        }
    }

    fn as_params(&self) -> [&(dyn postgres::types::ToSql + Sync); 91] {
        [
            &self.server_id, &self.id, &self.coin, &self.currency, &self.platform,
            &self.strategy_id, &self.server_db_id,
            &self.status, &self.sell_reason_code, &self.sell_reason, &self.is_short,
            &self.emulator, &self.from_cache, &self.job_is_done, &self.cancel_request,
            &self.server_forced_remove, &self.immune_for_clicks, &self.has_local_visual_order,
            &self.buy_price, &self.sell_price, &self.profit_btc,
            &self.vstop_on, &self.vstop_fixed, &self.vstop_level, &self.vstop_vol,
            &self.panic_sell, &self.is_moon_shot, &self.corridor_price_down, &self.corridor_price_up,
            &self.pending_buy_cond_price, &self.pending_cancel,
            &self.bulk_replace_buy, &self.bulk_replace_sell,
            // buy_*
            &self.buy_int_id, &self.buy_quantity, &self.buy_quantity_remaining,
            &self.buy_total_btc, &self.buy_spent_btc, &self.buy_open_time, &self.buy_close_time,
            &self.buy_actual_price, &self.buy_mean_price, &self.buy_quantity_base,
            &self.buy_actual_q, &self.buy_tmp_btc, &self.buy_create_time, &self.buy_panic_sell_down,
            &self.buy_order_type, &self.buy_sub_type, &self.buy_stop_flag, &self.buy_partial_done,
            &self.buy_leverage, &self.buy_is_opened, &self.buy_is_closed, &self.buy_canceled,
            &self.buy_is_short,
            // sell_*
            &self.sell_int_id, &self.sell_quantity, &self.sell_quantity_remaining,
            &self.sell_total_btc, &self.sell_spent_btc, &self.sell_open_time, &self.sell_close_time,
            &self.sell_actual_price, &self.sell_mean_price, &self.sell_quantity_base,
            &self.sell_actual_q, &self.sell_tmp_btc, &self.sell_create_time, &self.sell_panic_sell_down,
            &self.sell_order_type, &self.sell_sub_type, &self.sell_stop_flag, &self.sell_partial_done,
            &self.sell_leverage, &self.sell_is_opened, &self.sell_is_closed, &self.sell_canceled,
            &self.sell_is_short,
            // stops
            &self.stop_loss_on, &self.sl_fixed, &self.sl_level, &self.sl_spread,
            &self.trailing_on, &self.trailing_fixed, &self.trailing_level, &self.ts_spread,
            &self.use_take_profit, &self.take_profit, &self.take_profit_changed,
            // link
            &self.strategy_version_id,
        ]
    }
}

// SQL: 86 named columns + 86 placeholders + ON CONFLICT update всех полей кроме PK.
const SQL: &str = r#"
INSERT INTO orders (
    server_id, id, coin, currency, platform, strategy_id, server_db_id,
    status, sell_reason_code, sell_reason, is_short, emulator, from_cache,
    job_is_done, cancel_request, server_forced_remove, immune_for_clicks,
    has_local_visual_order,
    buy_price, sell_price, profit_btc,
    vstop_on, vstop_fixed, vstop_level, vstop_vol,
    panic_sell, is_moon_shot, corridor_price_down, corridor_price_up,
    pending_buy_cond_price, pending_cancel, bulk_replace_buy, bulk_replace_sell,
    buy_int_id, buy_quantity, buy_quantity_remaining, buy_total_btc, buy_spent_btc,
    buy_open_time, buy_close_time, buy_actual_price, buy_mean_price,
    buy_quantity_base, buy_actual_q, buy_tmp_btc, buy_create_time,
    buy_panic_sell_down, buy_order_type, buy_sub_type, buy_stop_flag,
    buy_partial_done, buy_leverage, buy_is_opened, buy_is_closed, buy_canceled,
    buy_is_short,
    sell_int_id, sell_quantity, sell_quantity_remaining, sell_total_btc,
    sell_spent_btc, sell_open_time, sell_close_time, sell_actual_price,
    sell_mean_price, sell_quantity_base, sell_actual_q, sell_tmp_btc,
    sell_create_time, sell_panic_sell_down, sell_order_type, sell_sub_type,
    sell_stop_flag, sell_partial_done, sell_leverage, sell_is_opened,
    sell_is_closed, sell_canceled, sell_is_short,
    stop_loss_on, sl_fixed, sl_level, sl_spread, trailing_on, trailing_fixed,
    trailing_level, ts_spread, use_take_profit, take_profit, take_profit_changed,
    strategy_version_id,
    updated_at
) VALUES (
    $1, $2, $3, $4, $5, $6, $7,
    $8, $9, $10, $11, $12, $13,
    $14, $15, $16, $17,
    $18,
    $19, $20, $21,
    $22, $23, $24, $25,
    $26, $27, $28, $29,
    $30, $31, $32, $33,
    $34, $35, $36, $37, $38,
    $39, $40, $41, $42,
    $43, $44, $45, $46,
    $47, $48, $49, $50,
    $51, $52, $53, $54, $55,
    $56,
    $57, $58, $59, $60,
    $61, $62, $63, $64,
    $65, $66, $67, $68,
    $69, $70, $71, $72,
    $73, $74, $75, $76,
    $77, $78, $79,
    $80, $81, $82, $83, $84, $85,
    $86, $87, $88, $89, $90,
    $91,
    NOW()
)
ON CONFLICT (server_id, id) DO UPDATE SET
    coin = EXCLUDED.coin,
    currency = EXCLUDED.currency,
    platform = EXCLUDED.platform,
    strategy_id = EXCLUDED.strategy_id,
    server_db_id = EXCLUDED.server_db_id,
    status = EXCLUDED.status,
    sell_reason_code = EXCLUDED.sell_reason_code,
    sell_reason = COALESCE(EXCLUDED.sell_reason, orders.sell_reason),
    is_short = EXCLUDED.is_short,
    emulator = EXCLUDED.emulator,
    from_cache = EXCLUDED.from_cache,
    job_is_done = EXCLUDED.job_is_done,
    cancel_request = EXCLUDED.cancel_request,
    server_forced_remove = EXCLUDED.server_forced_remove,
    immune_for_clicks = EXCLUDED.immune_for_clicks,
    has_local_visual_order = EXCLUDED.has_local_visual_order,
    buy_price = EXCLUDED.buy_price,
    sell_price = EXCLUDED.sell_price,
    profit_btc = EXCLUDED.profit_btc,
    vstop_on = EXCLUDED.vstop_on,
    vstop_fixed = EXCLUDED.vstop_fixed,
    vstop_level = EXCLUDED.vstop_level,
    vstop_vol = EXCLUDED.vstop_vol,
    panic_sell = EXCLUDED.panic_sell,
    is_moon_shot = EXCLUDED.is_moon_shot,
    corridor_price_down = EXCLUDED.corridor_price_down,
    corridor_price_up = EXCLUDED.corridor_price_up,
    pending_buy_cond_price = EXCLUDED.pending_buy_cond_price,
    pending_cancel = EXCLUDED.pending_cancel,
    bulk_replace_buy = EXCLUDED.bulk_replace_buy,
    bulk_replace_sell = EXCLUDED.bulk_replace_sell,
    buy_int_id = EXCLUDED.buy_int_id,
    buy_quantity = EXCLUDED.buy_quantity,
    buy_quantity_remaining = EXCLUDED.buy_quantity_remaining,
    buy_total_btc = EXCLUDED.buy_total_btc,
    buy_spent_btc = EXCLUDED.buy_spent_btc,
    buy_open_time = COALESCE(EXCLUDED.buy_open_time, orders.buy_open_time),
    buy_close_time = COALESCE(EXCLUDED.buy_close_time, orders.buy_close_time),
    buy_actual_price = EXCLUDED.buy_actual_price,
    buy_mean_price = EXCLUDED.buy_mean_price,
    buy_quantity_base = EXCLUDED.buy_quantity_base,
    buy_actual_q = EXCLUDED.buy_actual_q,
    buy_tmp_btc = EXCLUDED.buy_tmp_btc,
    buy_create_time = COALESCE(EXCLUDED.buy_create_time, orders.buy_create_time),
    buy_panic_sell_down = EXCLUDED.buy_panic_sell_down,
    buy_order_type = EXCLUDED.buy_order_type,
    buy_sub_type = EXCLUDED.buy_sub_type,
    buy_stop_flag = EXCLUDED.buy_stop_flag,
    buy_partial_done = EXCLUDED.buy_partial_done,
    buy_leverage = EXCLUDED.buy_leverage,
    buy_is_opened = EXCLUDED.buy_is_opened,
    buy_is_closed = EXCLUDED.buy_is_closed,
    buy_canceled = EXCLUDED.buy_canceled,
    buy_is_short = EXCLUDED.buy_is_short,
    sell_int_id = EXCLUDED.sell_int_id,
    sell_quantity = EXCLUDED.sell_quantity,
    sell_quantity_remaining = EXCLUDED.sell_quantity_remaining,
    sell_total_btc = EXCLUDED.sell_total_btc,
    sell_spent_btc = EXCLUDED.sell_spent_btc,
    sell_open_time = COALESCE(EXCLUDED.sell_open_time, orders.sell_open_time),
    sell_close_time = COALESCE(EXCLUDED.sell_close_time, orders.sell_close_time),
    sell_actual_price = EXCLUDED.sell_actual_price,
    sell_mean_price = EXCLUDED.sell_mean_price,
    sell_quantity_base = EXCLUDED.sell_quantity_base,
    sell_actual_q = EXCLUDED.sell_actual_q,
    sell_tmp_btc = EXCLUDED.sell_tmp_btc,
    sell_create_time = COALESCE(EXCLUDED.sell_create_time, orders.sell_create_time),
    sell_panic_sell_down = EXCLUDED.sell_panic_sell_down,
    sell_order_type = EXCLUDED.sell_order_type,
    sell_sub_type = EXCLUDED.sell_sub_type,
    sell_stop_flag = EXCLUDED.sell_stop_flag,
    sell_partial_done = EXCLUDED.sell_partial_done,
    sell_leverage = EXCLUDED.sell_leverage,
    sell_is_opened = EXCLUDED.sell_is_opened,
    sell_is_closed = EXCLUDED.sell_is_closed,
    sell_canceled = EXCLUDED.sell_canceled,
    sell_is_short = EXCLUDED.sell_is_short,
    stop_loss_on = EXCLUDED.stop_loss_on,
    sl_fixed = EXCLUDED.sl_fixed,
    sl_level = EXCLUDED.sl_level,
    sl_spread = EXCLUDED.sl_spread,
    trailing_on = EXCLUDED.trailing_on,
    trailing_fixed = EXCLUDED.trailing_fixed,
    trailing_level = EXCLUDED.trailing_level,
    ts_spread = EXCLUDED.ts_spread,
    use_take_profit = EXCLUDED.use_take_profit,
    take_profit = EXCLUDED.take_profit,
    take_profit_changed = EXCLUDED.take_profit_changed,
    strategy_version_id = COALESCE(EXCLUDED.strategy_version_id, orders.strategy_version_id),
    updated_at = NOW()
"#;

// ── Helpers ─────────────────────────────────────────────────────

const DELPHI_UNIX_EPOCH: f64 = 25569.0; // дней между 1899-12-30 и 1970-01-01

fn delphi_to_utc(delphi: f64) -> Option<DateTime<Utc>> {
    if delphi == 0.0 || !delphi.is_finite() {
        return None;
    }
    let unix_sec_f = (delphi - DELPHI_UNIX_EPOCH) * 86400.0;
    let secs = unix_sec_f.trunc() as i64;
    let nsecs = ((unix_sec_f.fract() * 1e9) as i64).max(0) as u32;
    Utc.timestamp_opt(secs, nsecs).single()
}

fn byte_to_bool(b: u8) -> bool {
    b != 0
}

fn sell_reason_text(code: u8) -> Option<String> {
    match code {
        0 => None,
        1 => Some("Sell Price".into()),
        2 => Some("Auto Price Down".into()),
        3 => Some("Sell Level".into()),
        4 => Some("Stop Loss".into()),
        5 => Some("Trailing".into()),
        _ => Some(format!("code={code}")),
    }
}

