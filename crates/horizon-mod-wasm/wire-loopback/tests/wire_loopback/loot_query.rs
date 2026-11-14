use super::*;
pub(super) fn guest_condition(event: &mut mod_api::LootConditionEvent<'_>) -> bool {
    if event.kind != "test:queried_parameters" {
        return false;
    }
    assert_eq!(event.ctx.queried_table_id, "minecraft:blocks/short_grass");
    let stack = event.ctx.queried_stack.as_ref().unwrap();
    assert_eq!(stack.count(), 3);
    assert_eq!(stack.damage(), 2);
    event.holds = true;
    true
}
#[test]
fn queried_parameters_reach_wasm_mod_condition() {
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let mut ctx = mod_api::LootContext::new(7);
    ctx.queried_table_id = "minecraft:blocks/short_grass".into();
    let mut stack = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 3).unwrap();
    stack.set_damage(2);
    ctx.queried_stack = Some(stack);
    let mut event = mod_api::LootConditionEvent {
        kind: "test:queried_parameters",
        data: &serde_json::Value::Null,
        ctx: &ctx,
        holds: false,
        tags: &Tags,
    };
    module.loot_condition(&mut event);
    assert!(event.holds);
}

struct Tags;
impl mod_api::Tags for Tags {}

pub(super) struct LootQueryHooks;
impl LootQueryHooks {
    pub(super) fn loot_condition(&self, event: &mut mod_api::LootConditionEvent<'_>) {
        if self::guest_condition(event) {
            return;
        }
        let tag = event
            .tags
            .key(&mod_api::Identifier::parse("test:ingredients").unwrap())
            .unwrap();
        let item = event.tags.items(tag).unwrap()[0];
        event.holds = event.tags.is_in_tag(item, tag).unwrap();
    }
}
