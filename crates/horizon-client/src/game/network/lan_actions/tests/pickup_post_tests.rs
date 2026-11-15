//! `item_picked_up_post` and `item_picked_up_partial` through the game's own pickup
//! paths, the host's player (`tick_dropped_items`) and a LAN guest (`guest_pickups`),
//! with literal counts.
use crate::game::Game;
use horizon_client::items::ItemStack;
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use mod_api::{
    Hooks, ItemPickedUpEvent, ItemPickedUpPartialEvent, ItemPickedUpPostEvent, ModEntry,
    ServerHost, TriState,
};
use mod_platform::testing::with_mods;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    /// `item_picked_up`'s answer.
    answer: TriState,
    /// A count the notice sets as the remainder.
    raise: Option<u32>,
    /// Whether the notice empties the remainder.
    empty: bool,
    /// Whether the notice removes every entity it can.
    kill: bool,
    /// The notices heard, in order.
    notices: Vec<Notice>,
    touched: Vec<(u32, Option<mod_api::ItemStack>)>,
    posted: Vec<u32>,
}

/// One notice heard: `"post"` or `"partial"`, player, original count, remainder count.
type Notice = (&'static str, u128, u32, Option<u32>);

#[derive(Clone, Default)]
struct Probe(Arc<Mutex<State>>);
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn item_picked_up(&self, host: &mut dyn ServerHost, event: &mut ItemPickedUpEvent<'_>) {
        let read = event.item.stack(host);
        let mut state = self.0.lock().unwrap();
        state.touched.push((event.item.entity().id(), read));
        *event.can_pickup = state.answer;
    }
    fn item_picked_up_post(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ItemPickedUpPostEvent<'_>,
    ) {
        self.0.lock().unwrap().posted.push(event.item.entity().id());
        self.heard(host, "post", event.player, event.original, event.remaining);
    }
    fn item_picked_up_partial(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ItemPickedUpPartialEvent<'_>,
    ) {
        self.heard(
            host,
            "partial",
            event.player,
            event.original,
            event.remaining,
        );
    }
}

impl Probe {
    /// Either notice: records it, then makes the case's changes.
    fn heard(
        &self,
        host: &mut dyn ServerHost,
        hook: &'static str,
        player: u128,
        original: &mod_api::ItemStack,
        remaining: &mut Option<mod_api::ItemStack>,
    ) {
        let (raise, empty, kill) = {
            let mut state = self.0.lock().unwrap();
            state.notices.push((
                hook,
                player,
                original.count(),
                remaining.as_ref().map(|rest| rest.count()),
            ));
            (state.raise, state.empty, state.kill)
        };
        if let Some(count) = raise {
            let mut rest = original.clone();
            rest.set_count(count).unwrap();
            *remaining = Some(rest);
        }
        if empty {
            *remaining = None;
        }
        if kill {
            // A command where one runs (a LAN host), else the entity handle's removal, which
            // the level sweeps out of its list later.
            let _ = host.commands().run_command("kill @e[type=minecraft:item]");
            let level = host.level("minecraft:overworld").unwrap();
            for entity in level.all_entities(host).unwrap_or_default() {
                let _ = entity.discard(host);
            }
        }
    }
}

/// One pickup case: the listener's setup, the inventory's room for apples,
/// and the ground stack's count and pickup delay.
struct Case {
    answer: TriState,
    raise: Option<u32>,
    empty: bool,
    kill: bool,
    room: u32,
    count: u32,
    delay: u32,
}

const CASE: Case = Case {
    answer: TriState::Default,
    raise: None,
    empty: false,
    kill: false,
    room: 8,
    count: 8,
    delay: 0,
};

/// What a case left: the notices heard, the apples on the ground (`None`
/// when the entity is gone) and the apples in the inventory's first slot.
type Outcome = (Vec<Notice>, Option<u32>, u32);

fn apples(room: u32) -> Option<ItemStack> {
    Some(match room {
        0 => ItemStack::new("minecraft:stone", 64),
        room => ItemStack::new("minecraft:apple", 64 - room),
    })
    .filter(|stack| stack.count > 0)
}

fn clear_ground(game: &mut Game<'_>) {
    while !game.dropped.is_empty() {
        game.remove_dropped(0);
    }
}

fn arm(probe: &Probe, case: &Case) {
    let mut state = probe.0.lock().unwrap();
    state.answer = case.answer;
    state.raise = case.raise;
    state.empty = case.empty;
    state.kill = case.kill;
    state.notices.clear();
}

fn spawn(game: &mut Game<'_>, at: [f64; 3], case: &Case) -> u32 {
    let level = game.level(game.dim.id()).unwrap();
    game.level_spawn_item(
        level,
        at[0],
        at[1],
        at[2],
        ItemStack::new("minecraft:apple", case.count),
        case.delay,
    )
    .unwrap()
    .id()
}

fn ground(game: &Game<'_>, id: u32) -> Option<u32> {
    game.dropped
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.stack.count)
}

fn with_probe(probe: &Probe, test: impl FnOnce()) {
    let mods: mod_api::Mods = vec![ModEntry::new("pickup_post", Box::new(probe.clone()))].into();
    with_mods(mods, test);
}

/// The host's player picks up through the lifted pass.
fn host_case(game: &mut Game<'_>, probe: &Probe, case: &Case) -> Outcome {
    clear_ground(game);
    game.inventory
        .fill(Some(ItemStack::new("minecraft:stone", 64)));
    game.offhand = Some(ItemStack::new("minecraft:stone", 64));
    game.inventory[0] = apples(case.room);
    arm(probe, case);
    let id = spawn(game, [1.5, 64.0, 1.5], case);
    let mut round = game.native_tick_round();
    game.tick_dropped_items(&mut round);
    game.native_removals_flush();
    let first = game.inventory[0].as_ref().map_or(0, |stack| {
        if &*stack.item == "minecraft:apple" {
            stack.count
        } else {
            0
        }
    });
    let notices = std::mem::take(&mut probe.0.lock().unwrap().notices);
    (notices, ground(game, id), first)
}

fn host_animations(game: &Game<'_>) -> u64 {
    game.pickups.made().get("item").copied().unwrap_or(0)
}

fn host_stat(game: &Game<'_>) -> i64 {
    game.stats
        .values
        .get("minecraft:picked_up")
        .and_then(|items| items.get("minecraft:apple"))
        .map_or(0, |&n| i64::from(n))
}

#[test]
#[ignore = "daily"]
fn host_pickup_tells_whole_and_partial_pickups_apart_and_keeps_listener_changes() {
    let probe = Probe::default();
    with_probe(&probe, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let scratch = std::path::PathBuf::from(
                std::env::var_os("HORIZON_HOME").expect("needs an isolated HORIZON_HOME"),
            )
            .join(format!("pickup-post-{}", std::process::id()));
            game.world_root = scratch.clone();
            game.enter_level(crate::game::Dim::Overworld).unwrap();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(1.5, 64.0, 1.5)).unwrap());
            game.level.player_uuid = Some(horizon_client::lan::playerdata::uuid_ints(7));
            game.lan_acting_uuid = Some(7);
            game.lan_acting_name = Some("pickup_post".into());

            // The whole stack goes in: one post notice, original 8, remainder empty.
            let (animations, stat) = (host_animations(game), host_stat(game));
            assert_eq!(
                host_case(game, &probe, &CASE),
                (vec![("post", 7, 8, None)], None, 64)
            );
            assert_eq!(host_animations(game), animations + 1);
            assert_eq!(host_stat(game), stat + 8);

            // 8 offered, 5 fit: one partial notice, original 8, remainder 3, and no post.
            let (animations, stat) = (host_animations(game), host_stat(game));
            let partial = Case { room: 5, ..CASE };
            assert_eq!(
                host_case(game, &probe, &partial),
                (vec![("partial", 7, 8, Some(3))], Some(3), 64)
            );
            assert_eq!(host_animations(game), animations + 1);
            assert_eq!(host_stat(game), stat + 5);

            // The partial listener raises the remainder to 6: the entity keeps 6, the pickup counts 2.
            let (animations, stat) = (host_animations(game), host_stat(game));
            let raised = Case {
                raise: Some(6),
                ..partial
            };
            assert_eq!(
                host_case(game, &probe, &raised),
                (vec![("partial", 7, 8, Some(3))], Some(6), 64)
            );
            assert_eq!(host_animations(game), animations + 1);
            assert_eq!(host_stat(game), stat + 2);

            // The partial listener empties the remainder: the entity goes, the pickup counts 8.
            let (animations, stat) = (host_animations(game), host_stat(game));
            let emptied = Case {
                empty: true,
                ..partial
            };
            assert_eq!(
                host_case(game, &probe, &emptied),
                (vec![("partial", 7, 8, Some(3))], None, 64)
            );
            assert_eq!(host_animations(game), animations + 1);
            assert_eq!(host_stat(game), stat + 8);

            // The partial listener kills the item, lent back to the level while it hears:
            // it stays gone, with no take animation, and the pickup counts 8.
            let (animations, stat) = (host_animations(game), host_stat(game));
            let killed = Case {
                kill: true,
                ..partial
            };
            assert_eq!(
                host_case(game, &probe, &killed),
                (vec![("partial", 7, 8, Some(3))], None, 64)
            );
            assert_eq!(host_animations(game), animations);
            assert_eq!(host_stat(game), stat + 8);

            // Refused by `item_picked_up`: nothing is told.
            let refused = Case {
                answer: TriState::False,
                ..CASE
            };
            assert_eq!(host_case(game, &probe, &refused), (vec![], Some(8), 56));

            // A full inventory takes nothing, even when `True` forces the pickup.
            let full = Case { room: 0, ..CASE };
            assert_eq!(host_case(game, &probe, &full), (vec![], Some(8), 0));
            let forced = Case {
                answer: TriState::True,
                room: 0,
                delay: 40,
                ..CASE
            };
            assert_eq!(host_case(game, &probe, &forced), (vec![], Some(8), 0));

            // A forced pickup that fits is told as the default one is.
            let forced = Case {
                answer: TriState::True,
                delay: 40,
                ..CASE
            };
            assert_eq!(
                host_case(game, &probe, &forced),
                (vec![("post", 7, 8, None)], None, 64)
            );

            // The listener puts 3 back: the entity stays with 3, the pickup counts 5.
            let (animations, stat) = (host_animations(game), host_stat(game));
            let raised = Case {
                raise: Some(3),
                ..CASE
            };
            assert_eq!(
                host_case(game, &probe, &raised),
                (vec![("post", 7, 8, None)], Some(3), 64)
            );
            assert_eq!(host_animations(game), animations + 1);
            assert_eq!(host_stat(game), stat + 5);

            // Creative takes the whole ground stack even with no inventory room.
            game.creative = true;
            let full = Case { room: 0, ..CASE };
            let (animations, stat) = (host_animations(game), host_stat(game));
            assert_eq!(
                host_case(game, &probe, &full),
                (vec![("post", 7, 8, None)], None, 0)
            );
            assert_eq!(host_animations(game), animations + 1);
            assert_eq!(host_stat(game), stat + 8);
            game.creative = false;

            // The post listener puts 3 back, then kills the item: it stays gone, with no panic
            // and no take animation.
            let animations = host_animations(game);
            let killed = Case {
                raise: Some(3),
                kill: true,
                ..CASE
            };
            assert_eq!(
                host_case(game, &probe, &killed),
                (vec![("post", 7, 8, None)], None, 64)
            );
            assert_eq!(host_animations(game), animations);
            let _ = std::fs::remove_dir_all(scratch);
        });
    });
}

/// Guest 12 picks up next to itself.
fn guest_case(game: &mut Game<'_>, probe: &Probe, case: &Case) -> Outcome {
    clear_ground(game);
    let room = case.room;
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(12, |guest| {
            guest
                .inventory
                .fill(Some(ItemStack::new("minecraft:stone", 64)));
            guest.offhand = Some(ItemStack::new("minecraft:stone", 64));
            guest.inventory[0] = apples(room);
        })
        .unwrap();
    arm(probe, case);
    let id = spawn(game, [3.0, 64.0, 5.0], case);
    let gi = game.guest_index(12).unwrap();
    game.guest_pickups(gi);
    let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
    let first = guest.inventory[0].as_ref().map_or(0, |stack| {
        if &*stack.item == "minecraft:apple" {
            stack.count
        } else {
            0
        }
    });
    let notices = std::mem::take(&mut probe.0.lock().unwrap().notices);
    (notices, ground(game, id), first)
}

#[test]
#[ignore = "daily"]
fn guest_pickup_tells_whole_and_partial_pickups_apart_and_keeps_listener_changes() {
    let probe = Probe::default();
    with_probe(&probe, || {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            // The whole stack goes in: one post notice naming guest 12, original 8.
            assert_eq!(
                guest_case(game, &probe, &CASE),
                (vec![("post", 12, 8, None)], None, 64)
            );

            // 8 offered, 5 fit: one partial notice, original 8, remainder 3, and no post.
            let partial = Case { room: 5, ..CASE };
            assert_eq!(
                guest_case(game, &probe, &partial),
                (vec![("partial", 12, 8, Some(3))], Some(3), 64)
            );

            // The partial listener raises the remainder to 6: the entity keeps 6.
            let raised = Case {
                raise: Some(6),
                ..partial
            };
            assert_eq!(
                guest_case(game, &probe, &raised),
                (vec![("partial", 12, 8, Some(3))], Some(6), 64)
            );

            // The partial listener empties the remainder: the entity goes.
            let emptied = Case {
                empty: true,
                ..partial
            };
            assert_eq!(
                guest_case(game, &probe, &emptied),
                (vec![("partial", 12, 8, Some(3))], None, 64)
            );

            // The partial listener kills the entity: it stays gone, with no panic.
            let killed = Case {
                kill: true,
                ..partial
            };
            assert_eq!(
                guest_case(game, &probe, &killed),
                (vec![("partial", 12, 8, Some(3))], None, 64)
            );

            // Refused by `item_picked_up`: nothing is told.
            let refused = Case {
                answer: TriState::False,
                ..CASE
            };
            assert_eq!(guest_case(game, &probe, &refused), (vec![], Some(8), 56));

            // A full inventory takes nothing, even when `True` forces the pickup.
            let full = Case { room: 0, ..CASE };
            assert_eq!(guest_case(game, &probe, &full), (vec![], Some(8), 0));
            let forced = Case {
                answer: TriState::True,
                room: 0,
                delay: 40,
                ..CASE
            };
            assert_eq!(guest_case(game, &probe, &forced), (vec![], Some(8), 0));

            // The listener puts 3 back: the entity stays with 3.
            let raised = Case {
                raise: Some(3),
                ..CASE
            };
            assert_eq!(
                guest_case(game, &probe, &raised),
                (vec![("post", 12, 8, None)], Some(3), 64)
            );

            // The listener puts 3 back, then kills the entity: it stays gone, with no panic.
            let killed = Case {
                raise: Some(3),
                kill: true,
                ..CASE
            };
            assert_eq!(
                guest_case(game, &probe, &killed),
                (vec![("post", 12, 8, None)], None, 64)
            );
        });
    });
}

/// Both pickup hooks name the ground item entity, and `item_picked_up`
/// reads its whole stack through the handle, components included.
#[test]
#[ignore = "daily"]
fn guest_pickup_hooks_name_the_ground_item_entity() {
    let probe = Probe::default();
    with_probe(&probe, || {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            clear_ground(game);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.inventory.fill(None))
                .unwrap();
            arm(&probe, &CASE);
            let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
            pick.damage = 17;
            pick.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("Named pick".into()));
            game.spawn_item(pick.clone(), [3.0, 64.0, 5.0], [0.0; 3], 0);
            let id = game.next_entity;
            let gi = game.guest_index(12).unwrap();
            game.guest_pickups(gi);
            let state = probe.0.lock().unwrap();
            assert_eq!(
                state.touched,
                vec![(id, Some(crate::game::api_values::test_snapshot(&pick)))]
            );
            assert_eq!(state.posted, vec![id]);
            assert_eq!(ground(game, id), None, "the pickaxe was taken");
        });
    });
}
