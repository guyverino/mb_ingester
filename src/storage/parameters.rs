//! Запись `StrategySchemaField` в таблицу `parameters` — полный raw-dump.
//!
//! Авто-заполняем ВСЕ поля доступные из moonproto-схемы. Curator-only поля
//! (min/max/step/description/depends_on/ignore_in_versioning) НЕ трогаем
//! при upsert'е — они правятся вручную и переживают ре-импорт.

use moonproto::commands::strategy_schema::{
    StrategyDynamicPicklist, StrategyFieldLayout, StrategyFieldType, StrategyFieldUiKind,
    StrategySchema, StrategySchemaField,
};
use moonproto::commands::strategy_serializer::FieldValue;
use postgres::Client;
use serde_json::{json, Value};

pub fn upsert_schema(client: &mut Client, schema: &StrategySchema) -> anyhow::Result<usize> {
    let kind_name_by_ordinal: std::collections::HashMap<u8, String> = schema
        .kinds
        .iter()
        .map(|k| (k.ordinal, k.name.clone()))
        .collect();

    let mut updated = 0usize;
    let mut tx = client.transaction()?;
    for f in &schema.fields {
        let row = ParamRow::from_field(f, &kind_name_by_ordinal);
        tx.execute(UPSERT_SQL, &row.as_params())?;
        updated += 1;
    }
    tx.commit()?;
    Ok(updated)
}

struct ParamRow {
    // Старые/основные:
    param_name: String,
    applicable_types: String,
    param_type: String,
    ui_kind: String,
    choices: Option<Value>,
    group_path: Option<String>,
    default_value: Option<String>,
    visible_kind_count: i32,

    // Raw байты протокола:
    raw_type_id: i16,
    type_name: String,
    raw_flags: i16,

    // Layout enum разобран:
    layout_kind: String,
    layout_value: Option<String>,
    chapter: Option<String>,

    // Picklist детально:
    has_static_picklist: bool,
    has_dynamic_picklist: bool,
    static_picklist_raw: Option<String>,
    dynamic_picklist_kind: Option<String>,
    dynamic_picklist_arg: Option<String>,

    // Видимость:
    visible_kind_mask: i64,
    visible_kind_ordinals: Vec<i16>,

    // Структурированный дефолт:
    default_value_json: Option<Value>,
}

impl ParamRow {
    fn from_field(
        f: &StrategySchemaField,
        kind_name_by_ordinal: &std::collections::HashMap<u8, String>,
    ) -> Self {
        let visible_kind_count = f.visible_kind_ordinals.len() as i32;

        let applicable_types = if f.visible_kind_ordinals.is_empty() {
            "*".to_string()
        } else {
            let mut names: Vec<&str> = f
                .visible_kind_ordinals
                .iter()
                .filter_map(|o| kind_name_by_ordinal.get(o).map(String::as_str))
                .collect();
            names.sort_unstable();
            names.dedup();
            if names.is_empty() {
                "*".to_string()
            } else {
                names.join(",")
            }
        };

        let ui_kind = match f.ui_kind {
            StrategyFieldUiKind::Edit => "Edit",
            StrategyFieldUiKind::Checkbox => "Checkbox",
            StrategyFieldUiKind::Combo => "Combo",
            StrategyFieldUiKind::Color => "Color",
            StrategyFieldUiKind::Unknown(_) => "Unknown",
        }
        .to_string();

        let type_name = f.type_id.name().to_string();

        let param_type = match f.ui_kind {
            StrategyFieldUiKind::Combo => "choice".to_string(),
            StrategyFieldUiKind::Color => "color".to_string(),
            StrategyFieldUiKind::Checkbox => "bool".to_string(),
            _ => match f.type_id {
                StrategyFieldType::Bool => "bool",
                StrategyFieldType::Int32
                | StrategyFieldType::Int64
                | StrategyFieldType::Byte
                | StrategyFieldType::Word
                | StrategyFieldType::UInt32
                | StrategyFieldType::UInt64 => "int",
                StrategyFieldType::Double | StrategyFieldType::Single => "float",
                StrategyFieldType::String => "string",
                StrategyFieldType::Unknown(_) => "unknown",
            }
            .to_string(),
        };

        let (layout_kind, layout_value, chapter, group_path) = match &f.layout {
            StrategyFieldLayout::None => ("None".to_string(), None, None, None),
            StrategyFieldLayout::Comment(c) => (
                "Comment".to_string(),
                Some(c.clone()),
                None,
                Some(c.clone()),
            ),
            StrategyFieldLayout::FilterClass(c) => (
                "FilterClass".to_string(),
                Some(c.clone()),
                None,
                Some(c.clone()),
            ),
            StrategyFieldLayout::ChapterClass { value, chapter } => (
                "ChapterClass".to_string(),
                Some(value.clone()),
                Some(chapter.clone()),
                Some(format!("{chapter}/{value}")),
            ),
        };

        let (has_static_picklist, static_picklist_raw, choices_static) =
            if f.static_picklist.is_empty() {
                (false, None, None)
            } else {
                (
                    true,
                    f.static_picklist_raw.clone(),
                    Some(Value::Array(
                        f.static_picklist
                            .iter()
                            .map(|s| Value::String(s.clone()))
                            .collect(),
                    )),
                )
            };

        let (has_dynamic_picklist, dynamic_picklist_kind, dynamic_picklist_arg, choices_dyn) =
            match &f.dynamic_picklist {
                None => (false, None, None, None),
                Some(StrategyDynamicPicklist::HookStrategies) => (
                    true,
                    Some("HookStrategies".to_string()),
                    None,
                    Some(json!({"dynamic": "HookStrategies"})),
                ),
                Some(StrategyDynamicPicklist::AllStrategies) => (
                    true,
                    Some("AllStrategies".to_string()),
                    None,
                    Some(json!({"dynamic": "AllStrategies"})),
                ),
                Some(StrategyDynamicPicklist::FieldName(name)) => (
                    true,
                    Some("FieldName".to_string()),
                    Some(name.clone()),
                    Some(json!({"dynamic": "FieldName", "arg": name})),
                ),
            };

        let choices = choices_static.or(choices_dyn);

        let visible_kind_mask = f.visible_kind_mask as i64;
        let visible_kind_ordinals: Vec<i16> =
            f.visible_kind_ordinals.iter().map(|&o| o as i16).collect();

        let default_value = f.default_value.as_ref().map(|v| format!("{v:?}"));
        let default_value_json = f.default_value.as_ref().map(field_value_to_json);

        Self {
            param_name: f.name.clone(),
            applicable_types,
            param_type,
            ui_kind,
            choices,
            group_path,
            default_value,
            visible_kind_count,
            raw_type_id: f.raw_type_id as i16,
            type_name,
            raw_flags: f.raw_flags as i16,
            layout_kind,
            layout_value,
            chapter,
            has_static_picklist,
            has_dynamic_picklist,
            static_picklist_raw,
            dynamic_picklist_kind,
            dynamic_picklist_arg,
            visible_kind_mask,
            visible_kind_ordinals,
            default_value_json,
        }
    }

    fn as_params(&self) -> [&(dyn postgres::types::ToSql + Sync); 22] {
        [
            &self.param_name,
            &self.applicable_types,
            &self.param_type,
            &self.ui_kind,
            &self.choices,
            &self.group_path,
            &self.default_value,
            &self.visible_kind_count,
            &self.raw_type_id,
            &self.type_name,
            &self.raw_flags,
            &self.layout_kind,
            &self.layout_value,
            &self.chapter,
            &self.has_static_picklist,
            &self.has_dynamic_picklist,
            &self.static_picklist_raw,
            &self.dynamic_picklist_kind,
            &self.dynamic_picklist_arg,
            &self.visible_kind_mask,
            &self.visible_kind_ordinals,
            &self.default_value_json,
        ]
    }
}

fn field_value_to_json(v: &FieldValue) -> Value {
    match v {
        FieldValue::Bool(b) => Value::Bool(*b),
        FieldValue::Int32(x) => Value::from(*x),
        FieldValue::Int64(x) => Value::from(*x),
        FieldValue::Double(x) => Value::from(*x),
        FieldValue::Single(x) => Value::from(*x as f64),
        FieldValue::String(s) => Value::String(s.clone()),
        FieldValue::Byte(x) => Value::from(*x),
        FieldValue::Word(x) => Value::from(*x),
        FieldValue::UInt32(x) => Value::from(*x),
        FieldValue::UInt64(x) => Value::from(*x),
    }
}

const UPSERT_SQL: &str = r#"
INSERT INTO parameters (
    param_name, applicable_types, param_type, ui_kind,
    choices, group_path, default_value, visible_kind_count,
    raw_type_id, type_name, raw_flags,
    layout_kind, layout_value, chapter,
    has_static_picklist, has_dynamic_picklist, static_picklist_raw,
    dynamic_picklist_kind, dynamic_picklist_arg,
    visible_kind_mask, visible_kind_ordinals, default_value_json,
    updated_at
) VALUES (
    $1, $2, $3, $4,
    $5, $6, $7, $8,
    $9, $10, $11,
    $12, $13, $14,
    $15, $16, $17,
    $18, $19,
    $20, $21, $22,
    NOW()
)
ON CONFLICT (param_name) DO UPDATE SET
    applicable_types      = EXCLUDED.applicable_types,
    param_type            = EXCLUDED.param_type,
    ui_kind               = EXCLUDED.ui_kind,
    choices               = EXCLUDED.choices,
    group_path            = COALESCE(parameters.group_path, EXCLUDED.group_path),
    default_value         = EXCLUDED.default_value,
    visible_kind_count    = EXCLUDED.visible_kind_count,
    raw_type_id           = EXCLUDED.raw_type_id,
    type_name             = EXCLUDED.type_name,
    raw_flags             = EXCLUDED.raw_flags,
    layout_kind           = EXCLUDED.layout_kind,
    layout_value          = EXCLUDED.layout_value,
    chapter               = EXCLUDED.chapter,
    has_static_picklist   = EXCLUDED.has_static_picklist,
    has_dynamic_picklist  = EXCLUDED.has_dynamic_picklist,
    static_picklist_raw   = EXCLUDED.static_picklist_raw,
    dynamic_picklist_kind = EXCLUDED.dynamic_picklist_kind,
    dynamic_picklist_arg  = EXCLUDED.dynamic_picklist_arg,
    visible_kind_mask     = EXCLUDED.visible_kind_mask,
    visible_kind_ordinals = EXCLUDED.visible_kind_ordinals,
    default_value_json    = EXCLUDED.default_value_json,
    updated_at            = NOW()
    -- min_value, max_value, step, description, depends_on, ignore_in_versioning, is_active
    -- НЕ обновляются — curator-only.
"#;
