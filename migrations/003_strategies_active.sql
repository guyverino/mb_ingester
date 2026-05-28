-- =============================================================
-- mb_ingester — strategies.active → checked + is_active
--
-- Старая колонка `active INTEGER` (legacy от Python-листенера, -1=on/0=off)
-- не соответствует MoonProto: в протоколе нет поля "Active" в fields-словаре.
-- Реальная семантика:
--   * checked    (bool)  — состояние UI-чекбокса (StrategySnapshot.checked)
--   * is_active  (bool)  — вычисленное по StrategyActiveMode::UsingMoonProto:
--                          checked && (can_auto_buy || run_detect_on_kernel)
--
-- Идемпотентно: ADD COLUMN IF NOT EXISTS, потом дроп старой `active`.
-- =============================================================

ALTER TABLE strategies
    ADD COLUMN IF NOT EXISTS checked   BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS is_active BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE strategies DROP COLUMN IF EXISTS active;

CREATE INDEX IF NOT EXISTS strategies_checked   ON strategies (server_id, checked)   WHERE checked;
CREATE INDEX IF NOT EXISTS strategies_is_active ON strategies (server_id, is_active) WHERE is_active;
