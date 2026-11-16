#![cfg(test)]
use crate::game::{native_entity_refs::tests::with_game, *};
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use horizon_core::geometry::Vec3;
use mod_api::{
    BlockPart, BlockPos, ClientHost, Direction, Flow, Hooks, ModEntry, PlayHitSoundEvent, Registrar,
};
use std::sync::{Arc, Mutex};

type Hits = Arc<Mutex<Vec<(BlockPos, Direction)>>>;
struct Quiet(Hits);
#[mod_api::horizon_part]
impl BlockPart for Quiet {
    fn play_hit_sound(&self, _: &mut dyn ClientHost, event: &mut PlayHitSoundEvent) -> Flow {
        self.0.lock().unwrap().push((event.pos, event.face));
        Flow::Cancel
    }
}
struct Register(Hits);
#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let block = r.block_key("minecraft:glass");
        r.block_part(block, Quiet(self.0.clone()));
    }
}

#[test]
#[ignore = "daily"]
fn play_hit_sound_helper_and_eight_mining_ticks() {
    for installed in [false, true] {
        let hits = Hits::default();
        let mods = if installed {
            vec![ModEntry::new("quiet", Box::new(Register(hits.clone())))]
        } else {
            Vec::new()
        };
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                game.save = Arc::new(horizon_client::save::SaveDir::new(
                    scratch.path().to_path_buf(),
                    horizon_client::save::LevelShape::OVERWORLD,
                ));
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 62.0, 4.5)).unwrap());
                game.player.as_mut().unwrap().ground.on_ground = true;
                game.creative = false;
                game.screen = Screen::Playing;
                game.title = None;
                game.left = true;
                game.forced_hit = Some(Hit {
                    pos: [5, 63, 4],
                    face: [-1, 0, 0],
                    point: [5.0, 63.5, 4.5],
                    inside: false,
                });
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let glass = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:glass")
                    .unwrap();
                for state in [stone, glass] {
                    game.audio.capture = Some(Vec::new());
                    hits.lock().unwrap().clear();
                    game.play_mining_hit_sound(state, [5, 63, 4], [-1, 0, 0], false);
                    assert_eq!(
                        game.audio.capture.as_ref().unwrap().len(),
                        usize::from(!(installed && state == glass))
                    );
                    if installed && state == glass {
                        assert_eq!(
                            *hits.lock().unwrap(),
                            [(BlockPos::new(5, 63, 4), Direction::West)]
                        );
                    }
                    if state == stone {
                        let sound = &game.audio.capture.as_ref().unwrap()[0];
                        assert_eq!(sound.event, "minecraft:block.stone.hit");
                        assert_eq!(sound.position, [5.5, 63.5, 4.5]);
                        assert_eq!((sound.volume, sound.pitch), (0.25, 0.5));
                        assert!(hits.lock().unwrap().is_empty());
                    }
                }
                for state in [stone, glass] {
                    game.audio.heard.clear();
                    hits.lock().unwrap().clear();
                    game.breaking_block_effects([5, 63, 4], [-1, 0, 0], false);
                    assert!(game.audio.heard.is_empty());
                    game.world.set(5, 63, 4, state).unwrap();
                    game.breaking_block_effects([5, 63, 4], [-1, 0, 0], true);
                    assert_eq!(
                        game.audio.heard.values().sum::<u32>(),
                        u32::from(!(installed && state == glass))
                    );
                }
                for state in [stone, glass] {
                    game.world.set(5, 63, 4, state).unwrap();
                    game.clicks = Default::default();
                    game.clicks.destroying.active = true;
                    game.clicks.destroying.item = game.main_hand_key();
                    game.clicks.destroying.pos = [5, 63, 4];
                    game.audio.capture = Some(Vec::new());
                    hits.lock().unwrap().clear();
                    let mut played = Vec::new();
                    for tick in 1..=8 {
                        game.tick_local_hand().unwrap();
                        for sound in game.audio.capture.as_mut().unwrap().drain(..) {
                            played.push((
                                tick,
                                sound.event,
                                sound.position,
                                sound.volume,
                                sound.pitch,
                            ));
                        }
                    }
                    if installed && state == glass {
                        assert!(played.is_empty());
                        assert_eq!(
                            *hits.lock().unwrap(),
                            [
                                (BlockPos::new(5, 63, 4), Direction::West),
                                (BlockPos::new(5, 63, 4), Direction::West)
                            ]
                        );
                    } else if state == stone {
                        assert_eq!(
                            played,
                            [
                                (
                                    1,
                                    "minecraft:block.stone.hit".into(),
                                    [5.5, 63.5, 4.5],
                                    0.25,
                                    0.5
                                ),
                                (
                                    5,
                                    "minecraft:block.stone.hit".into(),
                                    [5.5, 63.5, 4.5],
                                    0.25,
                                    0.5
                                )
                            ]
                        );
                    }
                }
            })
        });
    }
}
