//! Predicting item-on-block interactions on the client level, out of remote.rs,
//! which may not grow.
use super::super::Game;
use horizon_client::first_person::Hand;
use horizon_client::items::ItemStack;

impl Game<'_> {
    /// `FlintAndSteelItem.useOn` / `FireChargeItem.useOn` on this client's level:
    /// lighting an unlit campfire, candle or candle cake, or placing fire at the hit face.
    /// True when it succeeded (SUCCESS, swings and ends the click); false when
    /// placement is rejected (FAIL, no swing, ends the click).
    pub(super) fn predict_fire_starter(
        &mut self,
        stack: &ItemStack,
        hit: &super::super::Hit,
        hand: Hand,
        seq: i32,
    ) -> bool {
        let is_fire_charge = &*stack.item == "minecraft:fire_charge";
        let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let name = self.block_name(hit.pos);
        // TntBlock.useItemOn primes TNT unless sneaking with hands full.
        // Server priming sends TNT_PRIMED and can refuse priming; leave ignition
        // effects and item consumption to the server.
        let sneaking = self.remote_shift_down();
        let hands_full = self.stack_in(self.selected).is_some() || self.offhand.is_some();
        if !(sneaking && hands_full) && &*name == "minecraft:tnt" {
            return true;
        }
        if self.can_light(clicked, &name) {
            let registry = self.pack.assets.registry;
            if let Some(mut state) = registry.state(clicked).cloned() {
                state.properties.insert("lit".into(), "true".into());
                if let Ok(lit_state) = registry.resolve(&state) {
                    let pos =
                        horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
                    self.predict_blocks(&vec![(pos, lit_state)], seq);
                }
            }
            let at = [
                f64::from(hit.pos[0]) + 0.5,
                f64::from(hit.pos[1]) + 0.5,
                f64::from(hit.pos[2]) + 0.5,
            ];
            let sound = if is_fire_charge {
                "minecraft:item.firecharge.use"
            } else {
                "minecraft:item.flintandsteel.use"
            };
            let pitch =
                horizon_client::items::fire_starter_pitch(is_fire_charge, &mut self.world.random);
            self.audio.play(sound, Some(at), 1.0, pitch);
            if is_fire_charge && !self.creative {
                self.with_hand(hand, Game::consume_held);
            }
            return true;
        }
        let target = [
            hit.pos[0] + hit.face[0],
            hit.pos[1] + hit.face[1],
            hit.pos[2] + hit.face[2],
        ];
        if self.can_place_used_fire(target) {
            let pos = horizon_core::position::BlockPos::new(target[0], target[1], target[2]);
            let state = self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, pos));
            self.predict_blocks(&vec![(pos, state)], seq);
            let sound = horizon_client::items::fire_starter_sound(is_fire_charge);
            let pitch =
                horizon_client::items::fire_starter_pitch(is_fire_charge, &mut self.world.random);
            let target_at = [
                f64::from(target[0]) + 0.5,
                f64::from(target[1]) + 0.5,
                f64::from(target[2]) + 0.5,
            ];
            self.audio.play(sound, Some(target_at), 1.0, pitch);
            if is_fire_charge && !self.creative {
                self.with_hand(hand, Game::consume_held);
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{
        Audio, Game, Hit, Options, Progress, Settings, execution_role::ExecutionRole,
        load_resources,
    };
    use horizon_client::first_person::Hand;
    use horizon_client::items::ItemStack;

    #[test]
    #[ignore = "daily"]
    fn remote_item_use_tnt_predicts_success_without_consumption_or_sound() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                let candidate = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION);
                if candidate.exists() {
                    Some(candidate)
                } else {
                    let main_candidate = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../../horizon/.local/horizon-client")
                        .join(horizon_core::VANILLA_VERSION);
                    main_candidate.exists().then_some(main_candidate)
                }
            })
            .expect("needs prepared resources in HORIZON_TEST_RESOURCES");
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::Client;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        let (client, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(client, "RemoteItemUseTest").unwrap();
        let tnt_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:tnt")
            .unwrap();
        let pos = [0, 64, 0];
        let mut chunk = horizon_client::chunk::Chunk::empty(0, 0);
        chunk.set(0, 64, 0, tnt_state.0);
        game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
        game.creative = false;

        let hit = Hit {
            pos,
            face: [0, 1, 0],
            point: [0.5, 65.0, 0.5],
            inside: false,
        };

        // Fire charge on TNT: returns true, leaves consumption and sound to server.
        let fire_charge = ItemStack::new("minecraft:fire_charge", 1);
        game.inventory[0] = Some(fire_charge.clone());
        game.selected = 0;
        game.audio.heard.clear();
        assert!(game.predict_fire_starter(&fire_charge, &hit, Hand::Main, 1));
        assert_eq!(
            game.inventory[0].as_ref().map(|s| s.count),
            Some(1),
            "survival fire charge must not be consumed on client for TNT"
        );
        assert!(
            game.audio.heard.is_empty(),
            "TNT ignition sound must not play on client"
        );

        // Flint and steel on TNT: returns true without playing sound.
        let flint_steel = ItemStack::new("minecraft:flint_and_steel", 1);
        game.inventory[0] = Some(flint_steel.clone());
        game.audio.heard.clear();
        assert!(game.predict_fire_starter(&flint_steel, &hit, Hand::Main, 2));
        assert_eq!(
            game.inventory[0].as_ref().map(|s| s.count),
            Some(1),
            "flint and steel must remain in inventory"
        );
        assert!(
            game.audio.heard.is_empty(),
            "TNT ignition sound must not play on client"
        );
    }
}
