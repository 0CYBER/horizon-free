//! The dragon egg of a won fight: vanilla's on the first win, which `dragon_egg_form` may cancel
//! as before, and `dragon_fight_won`, asked on every win, which may place one or leave it.
use mod_api::{DragonEggFormEvent, DragonFightWonEvent, Flow, Hooks, LevelHost};
use std::sync::{Arc, Mutex};

type Heard<T> = Arc<Mutex<Vec<T>>>;

/// Hears `dragon_egg_form` as most mods do.
struct Plain(Heard<bool>);
#[mod_api::__subscriptions]
impl Hooks for Plain {
    fn dragon_egg_form(&self, _: &dyn LevelHost, event: &mut DragonEggFormEvent) -> Flow {
        self.0.lock().unwrap().push(event.cancelled);
        Flow::Continue
    }
}

/// Hears `dragon_egg_form` even when it is cancelled: it still hears the first win only.
struct Watcher(Heard<bool>);
#[mod_api::__subscriptions]
impl Hooks for Watcher {
    #[listener(receive_cancelled)]
    fn dragon_egg_form(&self, _: &dyn LevelHost, event: &mut DragonEggFormEvent) -> Flow {
        self.0.lock().unwrap().push(event.cancelled);
        Flow::Continue
    }
}

/// Places an egg on every later win, as More Dragon Eggs does.
struct EveryWin(Heard<(bool, bool)>);
#[mod_api::__subscriptions]
impl Hooks for EveryWin {
    fn dragon_fight_won(&self, _: &dyn LevelHost, event: &mut DragonFightWonEvent) -> Option<bool> {
        self.0.lock().unwrap().push((event.won_before, event.forms));
        event.won_before.then_some(true)
    }
}

/// The heights of the dragon eggs on the podium's column after two won fights.
fn eggs_after_two_wins(mods: mod_api::Mods) -> Vec<i32> {
    let mut eggs = Vec::new();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            for (x, z) in [(-1, -1), (-1, 0), (0, -1), (0, 0)] {
                game.world
                    .chunks
                    .insert((x, z), Arc::new(horizon_client::chunk::Chunk::empty(x, z)));
            }
            game.dragon_killed();
            game.dragon_killed();
            let egg = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dragon_egg")
                .unwrap();
            eggs = (0..256)
                .filter(|&y| game.world.block(0, y, 0) == egg)
                .collect();
        });
    });
    eggs
}

#[test]
#[ignore = "daily"]
fn every_win_asks_dragon_fight_won_and_dragon_egg_form_keeps_to_the_first() {
    // Vanilla: the egg on the podium's pillar (bedrock up to y 64 at the default rim) once.
    assert_eq!(eggs_after_two_wins(Vec::new().into()), [65]);

    let plain = Heard::default();
    let watcher = Heard::default();
    let every_win = Heard::default();
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("plain", Box::new(Plain(plain.clone()))),
        mod_api::ModEntry::new("watcher", Box::new(Watcher(watcher.clone()))),
        mod_api::ModEntry::new("every_win", Box::new(EveryWin(every_win.clone()))),
    ]
    .into();
    // The second egg lands on the first, as the game places the first on the pillar.
    assert_eq!(eggs_after_two_wins(mods), [65, 66]);
    assert_eq!(*plain.lock().unwrap(), [false], "the first win only");
    assert_eq!(
        *watcher.lock().unwrap(),
        [false],
        "the first win only, even receiving cancelled events"
    );
    assert_eq!(
        *every_win.lock().unwrap(),
        [(false, true), (true, false)],
        "every win, with what happens unless it answers"
    );
}
