-- =============================================================
-- mb_ingester — strategies широкая схема
--
-- Добавляем все колонки которые доступны из StrategySnapshot:
--   * raw fields структуры: strategy_ver, last_date, kind, path
--   * convenience-методы из moonproto:
--     can_auto_buy, auto_buy, run_detect_on_kernel, is_short,
--     sell_from_asset, sell_price_field
-- =============================================================

ALTER TABLE strategies
    ADD COLUMN IF NOT EXISTS strategy_ver         INTEGER,
    ADD COLUMN IF NOT EXISTS last_date_ms         BIGINT,             -- epoch ms (server-converted Delphi time)
    ADD COLUMN IF NOT EXISTS kind                 SMALLINT,           -- StrategyKind ordinal
    ADD COLUMN IF NOT EXISTS path                 TEXT,               -- folder path
    ADD COLUMN IF NOT EXISTS sell_price_field     DOUBLE PRECISION,   -- snapshot.sell_price_field()
    ADD COLUMN IF NOT EXISTS auto_buy             BOOLEAN,            -- snapshot.auto_buy()
    ADD COLUMN IF NOT EXISTS can_auto_buy         BOOLEAN,            -- snapshot.can_auto_buy() (auto_buy OR MoonShot kind, кроме MANUAL)
    ADD COLUMN IF NOT EXISTS run_detect_on_kernel BOOLEAN,            -- snapshot.run_detect_on_kernel()
    ADD COLUMN IF NOT EXISTS sell_from_asset      BOOLEAN,            -- snapshot.sell_from_asset()
    ADD COLUMN IF NOT EXISTS short                BOOLEAN;             -- snapshot.is_short() (legacy is_short поле тоже остаётся через extract_string)

CREATE INDEX IF NOT EXISTS strategies_kind ON strategies (kind);
CREATE INDEX IF NOT EXISTS strategies_path ON strategies (server_id, path);
