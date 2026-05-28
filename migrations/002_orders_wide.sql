-- =============================================================
-- mb_ingester — orders table widening
--
-- Заменяет минимальную orders (17 колонок) на полную копию `Order` из
-- moonproto: Order top-level + buy_order/sell_order (OrderCompact) + stops
-- (StopSettings). Всё что есть в протоколе по ордеру — в БД.
--
-- Идемпотентно через sentinel: если колонка `buy_int_id` уже существует,
-- значит таблица уже в широкой схеме, миграция no-op. Иначе — DROP+CREATE.
--
-- ВНИМАНИЕ: при первом применении эта миграция дропает существующую orders
-- (старые строки теряются). Это допустимо т.к. модель тоже изменилась
-- (теперь записываем только исполненные сделки status >= BuyDone).
-- =============================================================

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'orders' AND column_name = 'buy_int_id'
    ) THEN
        DROP TABLE IF EXISTS orders CASCADE;

        CREATE TABLE orders (
            -- ── Identity ───────────────────────────────────────────────
            server_id           INTEGER     NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
            id                  NUMERIC     NOT NULL,                  -- Order.uid (u64, может > i64::MAX)
            coin                TEXT,                                   -- Order.market_name
            currency            SMALLINT,                               -- Order.currency (u8)
            platform            SMALLINT,                               -- Order.platform (u8)
            strategy_id         NUMERIC,                                -- Order.strat_id (u64)
            server_db_id        INTEGER,                                -- Order.db_id (server-side DB id)

            -- ── Status & lifecycle ─────────────────────────────────────
            status              SMALLINT,                               -- Order.status (OrderWorkerStatus.0)
            sell_reason_code    SMALLINT,                               -- Order.sell_reason_code
            sell_reason         TEXT,                                   -- человекочитаемое имя SellReason
            is_short            BOOLEAN,
            emulator            BOOLEAN,
            from_cache          BOOLEAN,                                -- restored from server cache after reconnect
            job_is_done         BOOLEAN,                                -- terminal + awaits removal
            cancel_request      BOOLEAN,                                -- server requested cancellation
            server_forced_remove BOOLEAN,                               -- TOrderNotFound arrived
            immune_for_clicks   BOOLEAN,
            has_local_visual_order BOOLEAN,

            -- ── Computed top-level prices ──────────────────────────────
            buy_price           DOUBLE PRECISION,                       -- Order.buy_price (desired)
            sell_price          DOUBLE PRECISION,                       -- Order.sell_price (desired)
            profit_btc          DOUBLE PRECISION,                       -- sell.total_btc - buy.spent_btc if filled

            -- ── VStop / corridor / panic ───────────────────────────────
            vstop_on            BOOLEAN,
            vstop_fixed         BOOLEAN,
            vstop_level         DOUBLE PRECISION,
            vstop_vol           DOUBLE PRECISION,
            panic_sell          BOOLEAN,
            is_moon_shot        BOOLEAN,
            corridor_price_down REAL,
            corridor_price_up   REAL,

            -- ── Pending / replace ──────────────────────────────────────
            pending_buy_cond_price DOUBLE PRECISION,
            pending_cancel      BOOLEAN,
            bulk_replace_buy    BOOLEAN,
            bulk_replace_sell   BOOLEAN,

            -- ── Buy side (OrderCompact, 23 поля) ───────────────────────
            buy_int_id          BIGINT,
            buy_quantity        DOUBLE PRECISION,
            buy_quantity_remaining DOUBLE PRECISION,
            buy_total_btc       DOUBLE PRECISION,
            buy_spent_btc       DOUBLE PRECISION,
            buy_open_time       TIMESTAMPTZ,
            buy_close_time      TIMESTAMPTZ,
            buy_actual_price    DOUBLE PRECISION,
            buy_mean_price      DOUBLE PRECISION,
            buy_quantity_base   DOUBLE PRECISION,
            buy_actual_q        DOUBLE PRECISION,
            buy_tmp_btc         DOUBLE PRECISION,
            buy_create_time     TIMESTAMPTZ,
            buy_panic_sell_down REAL,
            buy_order_type      SMALLINT,
            buy_sub_type        SMALLINT,
            buy_stop_flag       SMALLINT,
            buy_partial_done    SMALLINT,
            buy_leverage        SMALLINT,
            buy_is_opened       BOOLEAN,
            buy_is_closed       BOOLEAN,
            buy_canceled        BOOLEAN,
            buy_is_short        BOOLEAN,

            -- ── Sell side (OrderCompact, 23 поля) ──────────────────────
            sell_int_id         BIGINT,
            sell_quantity       DOUBLE PRECISION,
            sell_quantity_remaining DOUBLE PRECISION,
            sell_total_btc      DOUBLE PRECISION,
            sell_spent_btc      DOUBLE PRECISION,
            sell_open_time      TIMESTAMPTZ,
            sell_close_time     TIMESTAMPTZ,
            sell_actual_price   DOUBLE PRECISION,
            sell_mean_price     DOUBLE PRECISION,
            sell_quantity_base  DOUBLE PRECISION,
            sell_actual_q       DOUBLE PRECISION,
            sell_tmp_btc        DOUBLE PRECISION,
            sell_create_time    TIMESTAMPTZ,
            sell_panic_sell_down REAL,
            sell_order_type     SMALLINT,
            sell_sub_type       SMALLINT,
            sell_stop_flag      SMALLINT,
            sell_partial_done   SMALLINT,
            sell_leverage       SMALLINT,
            sell_is_opened      BOOLEAN,
            sell_is_closed      BOOLEAN,
            sell_canceled       BOOLEAN,
            sell_is_short       BOOLEAN,

            -- ── Stops (StopSettings) ───────────────────────────────────
            stop_loss_on        BOOLEAN,
            sl_fixed            BOOLEAN,
            sl_level            DOUBLE PRECISION,
            sl_spread           DOUBLE PRECISION,
            trailing_on         BOOLEAN,
            trailing_fixed      BOOLEAN,
            trailing_level      DOUBLE PRECISION,
            ts_spread           DOUBLE PRECISION,
            use_take_profit     BOOLEAN,
            take_profit         DOUBLE PRECISION,
            take_profit_changed BOOLEAN,

            -- ── Bookkeeping ────────────────────────────────────────────
            first_seen_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

            PRIMARY KEY (server_id, id)
        );

        CREATE INDEX orders_strategy   ON orders (server_id, strategy_id);
        CREATE INDEX orders_market     ON orders (coin);
        CREATE INDEX orders_status     ON orders (status);
        CREATE INDEX orders_buy_open   ON orders (buy_open_time DESC) WHERE buy_open_time IS NOT NULL;
        CREATE INDEX orders_sell_close ON orders (sell_close_time DESC) WHERE sell_close_time IS NOT NULL;

        COMMENT ON TABLE orders IS
            'Журнал реальных сделок (только статусы >= BuyDone). Содержит полную копию Order из moonproto: top-level + buy_order/sell_order (OrderCompact) + stops (StopSettings).';
    END IF;
END
$$;
