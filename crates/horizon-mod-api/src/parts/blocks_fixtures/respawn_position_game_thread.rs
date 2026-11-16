/// Install respawn parts: a player respawning at glass stands two blocks above it, facing
/// yaw 10 and pitch 20, and a red bed is no respawn point.
pub fn respawn_position_game_thread() {
    struct Glass;
    #[mod_api::horizon_part]
    impl BlockPart for Glass {
        fn respawn_position(&self, _: &dyn Host, event: &mut RespawnPositionEvent<'_>) {
            assert_eq!(event.entity_type, "minecraft:player");
            let p = event.pos;
            let at = [
                f64::from(p.x) + 0.5,
                f64::from(p.y) + 2.0,
                f64::from(p.z) + 0.5,
            ];
            *event.position = Some((at, 10.0, 20.0));
        }
    }
    struct Bed;
    #[mod_api::horizon_part]
    impl BlockPart for Bed {
        fn respawn_position(&self, _: &dyn Host, event: &mut RespawnPositionEvent<'_>) {
            *event.position = None;
        }
    }
    struct RespawnMod;
    #[mod_api::__subscriptions]
    impl Hooks for RespawnMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let glass = r.block_key("minecraft:glass");
            r.block_part(glass, Glass);
            let bed = r.block_key("minecraft:red_bed");
            r.block_part(bed, Bed);
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("respawn_test", Box::new(RespawnMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}

std::thread_local! {
    static FALLING_TICKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}
