/// Install a part on snow, a block of layers: a player breaking more than one layer takes
/// one and leaves the block standing, dropping a snowball for it, and one layer is removed as
/// vanilla removes it. Glass is removed by the part itself, which answers that it was destroyed.
/// A shulker box and a tripwire are kept by their part (`Some(false)`), and stone's part
/// leaves the removal to vanilla while it records where its harvest is read
/// (`take_harvest_reads`), and a decorated pot's part records whether the state its harvest
/// is asked about is cracked (`take_pot_harvest_reads`).
pub fn destroyed_by_player_game_thread() {
    struct Layers;
    #[mod_api::horizon_part]
    impl BlockPart for Layers {
        fn on_destroyed_by_player(
            &self,
            host: &mut dyn ServerHost,
            event: &mut OnDestroyedByPlayerEvent<'_>,
        ) -> Option<bool> {
            record_destroy(event);
            let states = host.block_states();
            let got = event.state.property(&states, "layers");
            let layers: u32 = got.ok()??.parse().ok()?;
            if layers < 2 {
                return None;
            }
            let snowball = mod_vocab::__private::item_key("minecraft:snowball")?;
            let level = host.level("minecraft:overworld")?;
            host.set_block_property(event.pos, "layers", &(layers - 1).to_string());
            let _ = level.pop_resource(host, event.pos, ItemStack::of(snowball));
            Some(false)
        }
    }
    struct Remover;
    #[mod_api::horizon_part]
    impl BlockPart for Remover {
        fn on_destroyed_by_player(
            &self,
            host: &mut dyn ServerHost,
            event: &mut OnDestroyedByPlayerEvent<'_>,
        ) -> Option<bool> {
            record_destroy(event);
            let level = host.level("minecraft:overworld")?;
            level.remove_block(host, event.pos, false).then_some(true)
        }
    }
    struct Keep;
    #[mod_api::horizon_part]
    impl BlockPart for Keep {
        fn on_destroyed_by_player(
            &self,
            _: &mut dyn ServerHost,
            event: &mut OnDestroyedByPlayerEvent<'_>,
        ) -> Option<bool> {
            record_destroy(event);
            Some(false)
        }
    }
    struct Witness;
    #[mod_api::horizon_part]
    impl BlockPart for Witness {
        fn can_harvest_block(&self, host: HostSide<'_>, event: &mut CanHarvestBlockEvent) {
            let HostSide::Server(host) = host else {
                return;
            };
            let Some(level) = host.level("minecraft:overworld") else {
                return;
            };
            let there = level.block(host, event.pos) == Some(event.state.block());
            HARVEST_READS.with(|r| r.borrow_mut().push(there));
        }
        fn on_destroyed_by_player(
            &self,
            _: &mut dyn ServerHost,
            event: &mut OnDestroyedByPlayerEvent<'_>,
        ) -> Option<bool> {
            record_destroy(event);
            None
        }
    }
    struct PotWitness;
    #[mod_api::horizon_part]
    impl BlockPart for PotWitness {
        fn can_harvest_block(&self, host: HostSide<'_>, event: &mut CanHarvestBlockEvent) {
            let HostSide::Server(host) = host else {
                return;
            };
            let states = host.block_states();
            let cracked = event.state.property(&states, "cracked").ok().flatten() == Some("true");
            POT_READS.with(|r| r.borrow_mut().push(cracked));
        }
    }
    struct DestroyMod;
    #[mod_api::__subscriptions]
    impl Hooks for DestroyMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let snow = r.block_key("minecraft:snow");
            r.block_part(snow, Layers);
            let glass = r.block_key("minecraft:glass");
            r.block_part(glass, Remover);
            for block in [
                "minecraft:shulker_box",
                "minecraft:tripwire",
                "minecraft:ice",
            ] {
                let key = r.block_key(block);
                r.block_part(key, Keep);
            }
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, Witness);
            let pot = r.block_key("minecraft:decorated_pot");
            r.block_part(pot, PotWitness);
        }
    }
    DESTROYS.with(|r| r.borrow_mut().clear());
    HARVEST_READS.with(|r| r.borrow_mut().clear());
    POT_READS.with(|r| r.borrow_mut().clear());
    let mods: crate::Mods = vec![ModEntry::new("destroy_test", Box::new(DestroyMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
