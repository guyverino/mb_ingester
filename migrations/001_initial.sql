-- =============================================================
-- mb_ingester — initial schema
--
-- Минимальная схема для open-source data-ingester'а:
--   * подключается к одному или нескольким Moonbot-серверам через MoonProto
--   * принимает snapshot'ы стратегий и поток ордеров
--   * пишет всё в PostgreSQL
--
-- Архитектура «модулей»:
--   * Таблица `modules` — каталог доступных модулей (listener_orders,
--     listener_strategies, …). Содержит описание, категорию, deps.
--   * Таблица `servers.modules` (JSONB) — какие модули включены ДЛЯ
--     конкретного сервера. Пример: {"listener_orders": true,
--     "listener_strategies": true}.
--   * `app_settings.module` — ссылка из параметра на модуль (settings
--     этого модуля).
--
-- Так добавление нового модуля = одна строка в `modules` + код, который
-- его реализует. Никаких ALTER TABLE для каждой фичи.
--
-- Идемпотентная миграция (CREATE TABLE IF NOT EXISTS).
-- =============================================================

BEGIN;

-- ────────────────────────────────────────────────────────────────
-- modules: каталог функциональности
-- ────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS modules (
    name            TEXT        PRIMARY KEY,            -- 'listener_orders'
    description     TEXT        NOT NULL,
    category        TEXT        NOT NULL,               -- 'storage' | 'analytics' | 'integration' | 'automation'
    dependencies    TEXT[]      NOT NULL DEFAULT '{}',  -- ['listener_strategies'] и т.п.
    default_enabled BOOLEAN     NOT NULL DEFAULT FALSE, -- по умолчанию для нового сервера
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE  modules IS 'Каталог доступных модулей системы. Добавление модуля = INSERT сюда + код, который смотрит на флаг в servers.modules.';
COMMENT ON COLUMN modules.dependencies IS 'Имена других модулей, без которых этот не работает.';

-- ────────────────────────────────────────────────────────────────
-- servers: список Moonbot-инстансов
-- ────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS servers (
    id          SERIAL PRIMARY KEY,
    name        TEXT        NOT NULL UNIQUE,
    ip          TEXT        NOT NULL,
    port        INTEGER     NOT NULL,
    token       TEXT,                              -- MoonProto base64 key
    modules     JSONB       NOT NULL DEFAULT '{}', -- per-server enabled flags
    note        TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON COLUMN servers.modules IS
'Per-server JSONB словарь enabled-флагов модулей. Пример: {"listener_orders": true, "listener_strategies": true}. Имена ключей сверяются с modules.name.';
COMMENT ON COLUMN servers.token IS
'MoonProto exported key (base64). Получается из самого Moonbot UI.';

-- ────────────────────────────────────────────────────────────────
-- app_settings: runtime-конфиг
-- ────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS app_settings (
    key         TEXT        PRIMARY KEY,
    value       TEXT        NOT NULL,
    description TEXT,
    module      TEXT REFERENCES modules(name),  -- к какому модулю относится (NULL = глобальный)
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS app_settings_module ON app_settings (module);

COMMENT ON TABLE  app_settings IS 'Runtime-параметры. Читаются с TTL-кэшем, можно менять без рестарта.';
COMMENT ON COLUMN app_settings.module IS 'FK на modules.name. NULL = глобальный параметр (например, db_url не нужен здесь, он только в TOML).';

-- ────────────────────────────────────────────────────────────────
-- strategies: текущее состояние стратегии (одна строка на стратегию)
-- ────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS strategies (
    id          SERIAL PRIMARY KEY,
    server_id   INTEGER     NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    moonbot_id  NUMERIC     NOT NULL,                -- StrategySnapshot.strategy_id (u64)
    name        TEXT        NOT NULL,
    signal_type TEXT,
    active      INTEGER     NOT NULL DEFAULT 0,      -- -1=on, 0=off (Moonbot convention)
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (server_id, moonbot_id)
);

CREATE INDEX IF NOT EXISTS strategies_name ON strategies (server_id, name);

-- ────────────────────────────────────────────────────────────────
-- strategy_versions: история (новая строка при изменении LastEditDate)
-- ────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS strategy_versions (
    id           BIGSERIAL  PRIMARY KEY,
    strategy_id  INTEGER    NOT NULL REFERENCES strategies(id) ON DELETE CASCADE,
    version_date TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    raw_data     JSONB      NOT NULL,
    UNIQUE (strategy_id, version_date)
);

CREATE INDEX IF NOT EXISTS strategy_versions_date ON strategy_versions (strategy_id, version_date DESC);

-- ────────────────────────────────────────────────────────────────
-- orders: см. 002_orders_wide.sql (полная схема под структуру Order
-- из moonproto, миграция идемпотентна через sentinel).
-- ────────────────────────────────────────────────────────────────

-- ────────────────────────────────────────────────────────────────
-- Seed: каталог встроенных модулей и их default-настроек
-- ────────────────────────────────────────────────────────────────
INSERT INTO modules (name, description, category, dependencies, default_enabled) VALUES
('listener_strategies',
 'Принимает snapshot-ы стратегий через MoonProto и сохраняет в strategies + strategy_versions.',
 'storage', '{}', TRUE),
('listener_orders',
 'Принимает события ордеров через MoonProto и сохраняет в orders.',
 'storage', '{}', TRUE)
ON CONFLICT (name) DO NOTHING;

-- Default app_settings — runtime-параметры с привязкой к модулю либо global (module=NULL)
INSERT INTO app_settings (key, value, description, module) VALUES
-- Глобальные (без модуля)
('connect_timeout_secs', '15',
 'Таймаут подключения к Moonbot (сек) при connect/Init.', NULL),
('event_poll_ms', '500',
 'Период опроса событий внутри session-thread (мс).', NULL),
-- listener_orders
('orders_subscribe_snapshot', 'true',
 'Запросить первичный AllStatuses-snapshot при старте сессии.', 'listener_orders'),
-- listener_strategies
('strategies_subscribe_schema', 'true',
 'Запрашивать live StratSchema от сервера во время Init.', 'listener_strategies'),
('strategies_log_field_diff', 'false',
 'Логировать список изменённых полей при появлении новой версии (отладка).', 'listener_strategies')
ON CONFLICT (key) DO NOTHING;

COMMIT;
