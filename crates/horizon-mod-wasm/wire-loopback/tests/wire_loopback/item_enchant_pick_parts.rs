use super::*;
thread_local! {
    static SHARD: std::cell::Cell<Option<mod_api::ItemKey>> = const { std::cell::Cell::new(None) };
    static ASKED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// A part on the amethyst shard: sharpness is primary for it, the table's result gets damage
/// 3 per enchantment it was given, and a pick keeps it in slot 4 only.
struct Part;
impl mod_api::ItemPart for Part {
    fn item_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[
            Some(mod_api::HookId::IsPrimaryItemFor),
            Some(mod_api::HookId::ApplyEnchantments),
            Some(mod_api::HookId::IsNotReplaceableByPickAction),
        ]
    }
    fn is_primary_item_for(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::IsPrimaryItemForEvent<'_>,
    ) {
        ASKED.with_borrow_mut(|asked| asked.push(event.enchantment.to_owned()));
        event.primary = event.enchantment == "minecraft:sharpness";
    }
    fn apply_enchantments(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::ApplyEnchantmentsEvent<'_>,
    ) {
        let levels: u32 = event.enchantments.iter().map(|(_, level)| *level).sum();
        event.result.set_damage(levels);
    }
    fn is_not_replaceable_by_pick_action(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::IsNotReplaceableByPickActionEvent<'_>,
    ) {
        assert_eq!(event.player, 0x55);
        event.not_replaceable = event.slot == 4;
    }
}
pub(super) fn register(registrar: &mut dyn mod_api::Registrar) {
    let item = registrar.item_key("minecraft:amethyst_shard");
    SHARD.set(Some(item));
    registrar.item_part(item, Part);
}
#[test]
fn generated_guest_item_dispatch_writes_back_the_enchant_and_pick_answers() {
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    install_counter();
    let shard = mod_api::ItemStack::new(SHARD.get().unwrap(), 1).unwrap();
    for vanilla in [false, true] {
        let mut primary = vanilla;
        mod_api::dispatch_is_primary_item_for(&World, &shard, "minecraft:sharpness", &mut primary);
        assert!(primary);
        let mut primary = vanilla;
        mod_api::dispatch_is_primary_item_for(&World, &shard, "minecraft:smite", &mut primary);
        assert!(!primary);
    }
    assert_eq!(
        ASKED.with_borrow_mut(std::mem::take),
        ["minecraft:sharpness", "minecraft:smite"].repeat(2)
    );
    let list: Vec<(Arc<str>, u32)> = vec![
        ("minecraft:sharpness".into(), 2),
        ("minecraft:unbreaking".into(), 3),
    ];
    let mut result = shard.clone();
    mod_api::dispatch_apply_enchantments(&World, &shard, &list, &mut result);
    assert_eq!(result.damage(), 5);
    assert_eq!(result.count(), 1);
    for (slot, keep) in [(4, true), (5, false)] {
        for vanilla in [false, true] {
            let mut not_replaceable = vanilla;
            mod_api::dispatch_is_not_replaceable_by_pick_action(
                &World,
                0x55,
                &shard,
                slot,
                &mut not_replaceable,
            );
            assert_eq!(not_replaceable, keep);
        }
    }
}

pub(super) struct ItemEnchantPickPartsHooks;
impl ItemEnchantPickPartsHooks {
    pub(super) fn register(
        &self,
        registrar: &mut dyn mod_api::Registrar,
        cx: &mod_api::RegistrationContext,
    ) {
        self::register(registrar);
    }
}
