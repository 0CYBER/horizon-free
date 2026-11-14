use super::*;

pub(super) struct LootDropsHooks;
impl LootDropsHooks {
    pub(super) fn loot_drops(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LootDropsEvent<'_>,
    ) {
        assert_eq!(event.context.entity, Some((12, "test:mob")));
        assert_eq!(event.context.table.len(), 300);
        let source = event.context.damage_source.unwrap();
        assert_eq!(
            (source.damage_type, source.causing, source.direct),
            ("minecraft:arrow", 41, 73)
        );
        assert_eq!(source.position, Some([1.0, 2.0, 3.0]));
        assert!(event.context.tool.is_none());
        EXPECTED_LOOT_HAND.with(|expected| {
            assert_eq!(
                event.context.weapon_item(host),
                expected.borrow().clone(),
                "the recorded weapon crosses the generated hook boundary"
            );
            assert_eq!(
                host.entity(source.causing as u32)
                    .and_then(|e| host.living(e))
                    .and_then(|living| living.hand_item(host, mod_api::Hand::Main)),
                expected.borrow().clone(),
                "the complete live hand crosses the generated Host boundary"
            )
        });
        event.drops.clear();
    }
}
