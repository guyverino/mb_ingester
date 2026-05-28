//! Запись `StrategySchemaField` в таблицу `parameters`.
//!
//! Авто-заполняем поля доступные из moonproto-схемы. Curator-only поля
//! (min/max/step/description/depends_on) НЕ трогаем при upsert'е — они
//! заполняются вручную и должны переживать ре-импорт схемы.

use moonproto::commands::strategy_schema::{
    StrategyFieldType, StrategyFieldUiKind, StrategySchema, StrategySchemaField,
};
use postgres::Client;
use serde_json::Value;

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
        tx.execute(
            UPSERT_SQL,
            &[
                &row.param_name,
                &row.applicable_types,
                &row.param_type,
                &row.ui_kind,
                &row.choices,
                &row.group_path,
                &row.default_value,
                &row.visible_kind_count,
            ],
        )?;
        updated += 1;
    }
    tx.commit()?;
    Ok(updated)
}

struct ParamRow {
    param_name: String,
    applicable_types: String,
    param_type: String,
    ui_kind: String,
    choices: Option<Value>,
    group_path: Option<String>,
    default_value: Option<String>,
    visible_kind_count: i32,
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

        // param_type: бизнес-категория (как в старой generator_param_schemas).
        // UI-первичный сигнал (Combo→choice, Color→color), затем тип данных.
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

        let choices = build_choices_json(f);
        let group_path = layout_group(f);
        let default_value = f.default_value.as_ref().map(|v| format!("{v:?}"));

        Self {
            param_name: f.name.clone(),
            applicable_types,
            param_type,
            ui_kind,
            choices,
            group_path,
            default_value,
            visible_kind_count,
        }
    }
}

fn build_choices_json(f: &StrategySchemaField) -> Option<Value> {
    if !f.static_picklist.is_empty() {
        return Some(Value::Array(
            f.static_picklist
                .iter()
                .map(|s| Value::String(s.clone()))
                .collect(),
        ));
    }
    if let Some(dyn_pl) = &f.dynamic_picklist {
        return Some(Value::String(format!("{dyn_pl:?}")));
    }
    None
}

fn layout_group(f: &StrategySchemaField) -> Option<String> {
    use moonproto::commands::strategy_schema::StrategyFieldLayout;
    match &f.layout {
        StrategyFieldLayout::None => None,
        StrategyFieldLayout::Comment(c) => Some(c.clone()),
        StrategyFieldLayout::FilterClass(c) => Some(c.clone()),
        StrategyFieldLayout::ChapterClass { value, chapter } => {
            Some(format!("{chapter}/{value}"))
        }
    }
}

const UPSERT_SQL: &str = r#"
INSERT INTO parameters (
    param_name, applicable_types, param_type, ui_kind,
    choices, group_path, default_value, visible_kind_count, updated_at
) VALUES (
    $1, $2, $3, $4,
    $5, $6, $7, $8, NOW()
)
ON CONFLICT (param_name) DO UPDATE SET
    applicable_types   = EXCLUDED.applicable_types,
    param_type         = EXCLUDED.param_type,
    ui_kind            = EXCLUDED.ui_kind,
    choices            = EXCLUDED.choices,
    group_path         = COALESCE(parameters.group_path, EXCLUDED.group_path),
    default_value      = EXCLUDED.default_value,
    visible_kind_count = EXCLUDED.visible_kind_count,
    updated_at         = NOW()
    -- min_value, max_value, step, description, depends_on, ignore_in_versioning
    -- НЕ обновляются — это curator-only поля.
"#;
