//! `menu_transfer` through the generated host/guest dispatch against the same part called
//! natively: the part reads the player, menu, block, recipe, `max` and `dry_run` as sent, and
//! its answer (or None, which leaves the game's grid transfer) comes back the same.
use super::*;
use mod_api::{MenuPart, MenuTransferEvent, RecipeTransferResult};
struct World;
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World);
impl mod_api::ServerHost for World {}
mod_api::impl_server_host_areas!(World);
/// A press's menu: its own recipes move (a dry run finds items missing), another recipe is
/// left to the grid transfer, and a menu with no block takes nothing.
struct Press;
#[mod_api::horizon_part]
impl MenuPart for Press {
    fn menu_transfer(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut MenuTransferEvent<'_>,
    ) -> Option<RecipeTransferResult> {
        if event.menu != "test:press" || event.player != 42 {
            return None;
        }
        if event.pos.is_none() {
            return Some(RecipeTransferResult::NotIntoMenu);
        }
        if !event.recipe.starts_with("test:press/") {
            return None;
        }
        Some(match (event.dry_run, event.max) {
            (true, _) => RecipeTransferResult::Missing,
            (false, true) => RecipeTransferResult::NoRoom,
            (false, false) => RecipeTransferResult::Moves,
        })
    }
}
#[test]
fn menu_transfer_answers_match_native_through_the_wire() {
    install_counter();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let proxy = WasmHandler {
        index: guest::add_handler(vec![guest::view::<dyn MenuPart>(std::sync::Arc::new(
            Press,
        ))]),
        module: WasmMod {
            group: 0,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        },
    };
    let mut answers = Vec::new();
    for (player, menu, pos) in [
        (42, "test:press", Some([3, 64, -9])),
        (42, "test:press", None),
        (7, "test:press", Some([3, 64, -9])),
        (42, "test:other", Some([3, 64, -9])),
    ] {
        for recipe in ["test:press/gear", "minecraft:stick"] {
            for (max, dry_run) in [(false, false), (true, false), (false, true)] {
                let ask = |part: &dyn MenuPart| {
                    let mut event = MenuTransferEvent {
                        player,
                        menu,
                        pos,
                        recipe,
                        max,
                        dry_run,
                    };
                    part.menu_transfer(&mut World, &mut event)
                };
                let (wasm, direct) = (ask(&proxy), ask(&Press));
                assert_eq!(
                    wasm, direct,
                    "{player} {menu} {pos:?} {recipe} {max} {dry_run}"
                );
                answers.push(direct);
            }
        }
    }
    use RecipeTransferResult as R;
    assert_eq!(
        answers[..6],
        [
            Some(R::Moves),
            Some(R::NoRoom),
            Some(R::Missing),
            None,
            None,
            None
        ]
    );
    assert_eq!(answers[6..12], [Some(R::NotIntoMenu); 6]);
    assert!(answers[12..].iter().all(Option::is_none));
}
