//! Species-specific entity tracker metadata.
use horizon_client::mobs::Extra;
use horizon_protocol::entity_data::{Value, serializer};

/// Enderman carried block, creepy and stared-at data, including reset defaults.
pub(super) fn enderman(
    extra: &Extra,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
    if let Some(state) = extra.carried {
        meta.push((
            16,
            serializer::OPTIONAL_BLOCK_STATE,
            Value::Id(state.0 as i32),
        ));
    }
    if extra.flag {
        meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
    }
    defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
    if extra.flag2 {
        meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
    }
    defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
    defaults.push((16, serializer::OPTIONAL_BLOCK_STATE, Value::Id(0)));
}
