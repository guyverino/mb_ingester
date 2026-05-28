-- =============================================================
-- mb_ingester — добавляем ВСЕ raw-поля из StrategySchemaField
--
-- Цель: полностью отразить структуру поля как её даёт moonproto,
-- чтобы можно было разобраться "что в каком виде приходит".
-- =============================================================

ALTER TABLE parameters
    -- Низкоуровневые байты (из wire-формата):
    ADD COLUMN IF NOT EXISTS raw_type_id           SMALLINT,            -- TID_BOOL=1, TID_INT32=4, etc
    ADD COLUMN IF NOT EXISTS type_name             TEXT,                -- Bool/Int32/Int64/Double/Single/String/Byte/Word/UInt32/UInt64
    ADD COLUMN IF NOT EXISTS raw_flags             SMALLINT,            -- bits 0-1=ui_kind, 4=has_static, 5=has_dynamic, 6=default_nz

    -- Разбор layout enum (None/Comment/FilterClass/ChapterClass):
    ADD COLUMN IF NOT EXISTS layout_kind           TEXT,                -- 'None'|'Comment'|'FilterClass'|'ChapterClass'
    ADD COLUMN IF NOT EXISTS layout_value          TEXT,                -- Comment text / FilterClass name / ChapterClass.value
    ADD COLUMN IF NOT EXISTS chapter               TEXT,                -- ChapterClass.chapter only

    -- Picklist детально:
    ADD COLUMN IF NOT EXISTS has_static_picklist   BOOLEAN,
    ADD COLUMN IF NOT EXISTS has_dynamic_picklist  BOOLEAN,
    ADD COLUMN IF NOT EXISTS static_picklist_raw   TEXT,                -- "A|B|C" из Delphi WriteStr16
    ADD COLUMN IF NOT EXISTS dynamic_picklist_kind TEXT,                -- 'HookStrategies'|'AllStrategies'|'FieldName'
    ADD COLUMN IF NOT EXISTS dynamic_picklist_arg  TEXT,                -- только для FieldName(name)

    -- Видимость:
    ADD COLUMN IF NOT EXISTS visible_kind_mask     BIGINT,              -- u32 bitset по ordinal
    ADD COLUMN IF NOT EXISTS visible_kind_ordinals SMALLINT[],          -- сам список ordinal'ов

    -- Дефолт как структурированный JSON (наряду с текущим default_value:TEXT через Debug):
    ADD COLUMN IF NOT EXISTS default_value_json    JSONB;

CREATE INDEX IF NOT EXISTS parameters_type_name ON parameters (type_name);
CREATE INDEX IF NOT EXISTS parameters_layout_kind ON parameters (layout_kind);
