use super::*;

impl HitLevel<'_> {
    /// `Block.fallOn`: entity `entity` of type `kind` landed on `state` at `pos` after falling
    /// `distance` blocks. `damage` is the fall damage `causeFallDamage` calculated, before hurt
    /// handling and the fall sounds: change it, and zero prevents both. A creative or
    /// fall-immune entity is told with zero damage and keeps its immunity. Players and mobs
    /// land here; dropped items also land with zero damage. Generated dispatch stages
    /// every requested API LandingWrite before committing damage or the engine write
    /// queue; an invalid state refuses the whole callback's output.
    pub(in crate::game) fn native_fall_on(&mut self, call: super::fire::FallOnCall<'_>) {
        self.fire_fall_on(call);
    }
    /// `Block.bounceOn`: entity `entity` of type `kind` bounced off `state` at `pos` and fell
    /// `fall_distance` blocks. `movement` is the movement the bounce leaves it, before
    /// vanilla sets it: change it to change what the entity keeps. Players and mobs bounce
    /// here; dropped items and the other entities do not yet.
    pub(in crate::game) fn native_bounce_on(&mut self, call: super::fire::BounceOnCall<'_>) {
        self.fire_bounce_on(call);
    }
}

#[cfg(test)]
mod landing_crossing_tests {
    use super::*;
    use crate::game::api_values::TryIntoApi;
    use mod_api::{BlockPart, FallOnEvent, Hooks, LevelRandomHost, ModEntry, Registrar};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct LandingPart(Arc<Mutex<Option<(mod_api::BlockState, bool)>>>);
    #[mod_api::horizon_part]
    impl BlockPart for LandingPart {
        fn fall_on(&self, _: &mut dyn LevelRandomHost, event: &mut FallOnEvent<'_>) {
            let (state, invalid) = self.0.lock().unwrap().unwrap();
            event.damage = 99.0;
            event.request(mod_api::LandingWrite::SetBlock(state));
            if invalid {
                event.request(mod_api::LandingWrite::SetBlock(
                    mod_api::BlockStateRecord::new(state.block(), u16::MAX).into(),
                ));
            }
        }
    }
    struct LandingMod(LandingPart);
    #[mod_api::__subscriptions]
    impl Hooks for LandingMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, self.0.clone());
        }
    }

    #[test]
    #[ignore = "daily"]
    fn landing_crossing_refuses_invalid_writes_and_sibling_damage() {
        let part = LandingPart::default();
        let mods: mod_api::Mods = vec![ModEntry::new(
            "landing_atomic",
            Box::new(LandingMod(part.clone())),
        )]
        .into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let state = stone.try_into_api().unwrap();
                let original =
                    vec![horizon_world::level::LandingWrite::DestroyBlock { drop: false }];
                for invalid in [false, true] {
                    *part.0.lock().unwrap() = Some((state, invalid));
                    let mut damage = 7.0;
                    let mut writes = original.clone();
                    let mut level = horizon_client::health::TestLevel::new(0);
                    HitLevel::new(level.level()).native_fall_on(super::super::fire::FallOnCall {
                        dimension: "minecraft:overworld",
                        entity: 1,
                        kind: "minecraft:player",
                        state: stone,
                        pos: BlockPos::new(1, 64, 1),
                        distance: 5.0,
                        damage: &mut damage,
                        writes: &mut writes,
                    });
                    if invalid {
                        assert_eq!(damage, 7.0, "invalid write must not commit sibling damage");
                        assert_eq!(writes, original, "valid prefix must not reach engine queue");
                    } else {
                        assert_eq!(damage, 99.0, "the real registered callback fired");
                        assert_eq!(
                            writes,
                            vec![
                                original[0],
                                horizon_world::level::LandingWrite::SetBlock(stone)
                            ]
                        );
                    }
                }
            });
        });
    }
}
