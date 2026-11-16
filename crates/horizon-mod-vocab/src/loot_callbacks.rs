//! Read-only capabilities for custom loot evaluation.
use crate::{LootContext, LootStack, Tags};
use serde_json::Value;

/// Dispatch a custom loot function with the current authoritative tags view.
pub fn native_loot_function(
    kind: &str,
    data: &Value,
    ctx: &LootContext,
    stack: &mut LootStack,
    tags: &dyn Tags,
) {
    crate::dispatch_loot_function(kind, data, ctx, tags, stack);
}

/// Dispatch a custom loot condition with the current authoritative tags view.
pub fn native_loot_condition(
    kind: &str,
    data: &Value,
    ctx: &LootContext,
    holds: &mut bool,
    tags: &dyn Tags,
) {
    crate::dispatch_loot_condition(kind, data, ctx, tags, holds);
}
