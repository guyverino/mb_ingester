-- =============================================================
-- mb_ingester — связь orders → strategy_versions
--
-- Каждый ордер фиксирует версию стратегии, которая была актуальна
-- в момент исполнения. Позволяет потом смотреть "по каким параметрам
-- сработал этот ордер".
--
-- Заполнение: при upsert ордера (status >= BuyDone) находим текущую
-- последнюю version_id для (server_id, strategy_id) и записываем.
-- ON DELETE SET NULL — если кто-то почистит strategy_versions, ордер
-- остаётся в журнале с null версией.
-- =============================================================

ALTER TABLE orders
    ADD COLUMN IF NOT EXISTS strategy_version_id BIGINT
        REFERENCES strategy_versions(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS orders_strategy_version
    ON orders (strategy_version_id)
    WHERE strategy_version_id IS NOT NULL;
