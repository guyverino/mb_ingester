-- =============================================================
-- mb_ingester — parameters: убираем min/max/step
--
-- Эти три колонки были наследием старой generator_param_schemas
-- (ручная курация для генератора). В новой системе их не используем.
--
-- Импорт description/depends_on из старой mb_ai-БД делается одноразово
-- через scripts/import_legacy_params.sql (вне ingester subtree, чтобы
-- не светить креды в публичный репо).
-- =============================================================

ALTER TABLE parameters DROP COLUMN IF EXISTS min_value;
ALTER TABLE parameters DROP COLUMN IF EXISTS max_value;
ALTER TABLE parameters DROP COLUMN IF EXISTS step;
