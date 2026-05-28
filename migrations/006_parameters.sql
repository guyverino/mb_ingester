-- =============================================================
-- mb_ingester — таблица описаний параметров стратегий
--
-- Источник:
--   * Авто-заполняемые поля из moonproto StrategySchema:
--     param_name, param_type, applicable_types, choices, group_path,
--     default_value, ui_kind
--   * Куратор/ручные поля (NULL по умолчанию, не перезаписываются upsert'ом
--     из протокола):
--     min_value, max_value, step, description, depends_on
--
-- ignore_in_versioning=TRUE — параметр не учитывается при определении
-- новой версии в strategy_versions (для UI-only полей: Active, AutoBuy,
-- RunDetectOnKernel, цветовых, звуковых и т.п.). Сейчас versioning
-- работает через LastEditDate, этот флаг информационный + задел на
-- будущее field-level diff versioning.
-- =============================================================

CREATE TABLE IF NOT EXISTS parameters (
    param_name           TEXT        PRIMARY KEY,
    applicable_types     TEXT        NOT NULL,         -- список kind names или '*'
    param_type           TEXT        NOT NULL,         -- 'bool' | 'int' | 'float' | 'string' | 'choice' | 'color'
    ui_kind              TEXT,                          -- raw из moonproto: Edit | Checkbox | Combo | Color

    -- Curator-only поля (заполняются вручную, upsert из схемы их не трогает):
    min_value            REAL,
    max_value            REAL,
    step                 REAL,
    description          TEXT,
    depends_on           TEXT,

    -- Auto из moonproto:
    choices              JSONB,                         -- static_picklist или dynamic_picklist
    group_path           TEXT,                          -- layout.Comment / FilterClass / ChapterClass
    default_value        TEXT,                          -- field.default_value сериализовано
    visible_kind_count   INTEGER,                       -- сколько kinds используют этот параметр

    -- Метаданные:
    is_active            BOOLEAN     NOT NULL DEFAULT TRUE,
    ignore_in_versioning BOOLEAN     NOT NULL DEFAULT FALSE,

    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS parameters_group ON parameters (group_path);
CREATE INDEX IF NOT EXISTS parameters_ignored ON parameters (ignore_in_versioning) WHERE ignore_in_versioning;

COMMENT ON TABLE parameters IS
    'Описания параметров стратегий. Часть полей авто-заполняется из moonproto StrategySchema, часть (min/max/step/description) — ручная курация.';
COMMENT ON COLUMN parameters.ignore_in_versioning IS
    'Если TRUE, изменение параметра не порождает новую запись в strategy_versions (UI-only / runtime-флаги).';

-- Заранее помечаем известные runtime/UI-параметры как игнорируемые для версионирования.
INSERT INTO parameters (param_name, applicable_types, param_type, ignore_in_versioning, description) VALUES
    ('Active',                   '*', 'bool',   TRUE, 'Runtime-флаг старт/стоп стратегии. Не параметр.'),
    ('AutoBuy',                  '*', 'bool',   TRUE, 'Runtime-флаг автоматических покупок.'),
    ('RunDetectOnKernel',        '*', 'bool',   TRUE, 'Runtime-флаг работы детекта на ядре.'),
    ('Checked',                  '*', 'bool',   TRUE, 'Состояние галочки в UI.'),
    ('SellOrderColor',           '*', 'color',  TRUE, 'Цвет ордера на продажу в UI.'),
    ('BuyOrderColor',            '*', 'color',  TRUE, 'Цвет ордера на покупку в UI.'),
    ('SoundKind',                '*', 'choice', TRUE, 'Звуковое уведомление в UI.'),
    ('SilentNoCharts',           '*', 'bool',   TRUE, 'Скрывать ли стратегию без графиков (UI).'),
    ('ReportTradesToTelegram',   '*', 'bool',   TRUE, 'Отправлять ли отчёты в Telegram (UI/integration).'),
    ('KeepAlert',                '*', 'int',    TRUE, 'Время удержания алерта в UI.')
ON CONFLICT (param_name) DO NOTHING;
