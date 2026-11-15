use super::*;
use crate::game::{
    native_entity_refs::tests::run_fresh_cases, native_guest_turn_tests::with_guests,
};
use mod_api::{
    AdvancementProgress, DataPackFilesEvent, Hooks, ModEntry, Mods, PlayerAdvancementProgressEvent,
    ServerHost,
};
use std::sync::Mutex;

const ID: &str = "testmod:progress";
const DATE: &str = "2020-01-02 03:04:05 +0000";

#[derive(Debug)]
struct Observation {
    player: u128,
    criterion: String,
    kind: ProgressType,
    progress: AdvancementProgress,
    rewarded: Option<bool>,
}

struct Observer(Arc<Mutex<Vec<Observation>>>);
#[mod_api::__subscriptions]
impl Hooks for Observer {
    fn data_pack_files(&self, _: &mut dyn ServerHost, event: &mut DataPackFilesEvent<'_>) {
        if matches!(
            std::env::var("HORIZON_PROGRESS_SNAPSHOT_CHILD").as_deref(),
            Ok("command-reload" | "nested-reload")
        ) {
            event.files.push((
                "advancement/testmod:progress".into(),
                br#"{
                "criteria": {
                    "alpha": {"trigger": "minecraft:tick"},
                    "gamma": {"trigger": "minecraft:impossible"}
                },
                "requirements": [["alpha"], ["gamma"]],
                "rewards": {"recipes": ["testmod:reward"], "experience": 7,
                    "loot": ["testmod:reward"], "function": "testmod:reward"}
            }"#
                .to_vec(),
            ));
            event.files.push((
                "recipe/testmod:reward".into(),
                br#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:diamond","count":1}}"#.to_vec(),
            ));
            event.files.push((
                "loot_table/testmod:reward".into(),
                br#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stone"}]}]}"#.to_vec(),
            ));
        }
    }

    fn player_advancement_progress(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerAdvancementProgressEvent<'_>,
    ) {
        if event.advancement != ID {
            return;
        }
        // Read through the Host during dispatch: no progress storage borrow is retained.
        let rewarded = host
            .player(event.player)
            .and_then(|player| player.has_discovered_recipe(host, "testmod:reward"));
        self.0.lock().unwrap().push(Observation {
            player: event.player,
            criterion: event.criterion.to_owned(),
            kind: event.progress_type,
            progress: event.progress.clone(),
            rewarded,
        });
    }
}

fn assert_progress(progress: &AdvancementProgress, obtained: [bool; 3], groups: u32, done: bool) {
    assert_eq!(
        progress
            .criteria
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta", "gamma"]
    );
    assert_eq!(
        progress
            .criteria
            .iter()
            .map(|c| c.obtained_at.is_some())
            .collect::<Vec<_>>(),
        obtained
    );
    assert_eq!(
        progress.requirements,
        vec![vec!["alpha", "beta"], vec!["gamma"]]
    );
    assert_eq!(progress.completed_requirements, groups);
    assert_eq!(progress.done, done);
}

#[test]
#[ignore = "daily"]
fn progress_events_capture_each_players_full_mutation_state() {
    if run_fresh_cases(
        "game::advancements::progress_tests::progress_events_capture_each_players_full_mutation_state",
        "HORIZON_PROGRESS_SNAPSHOT_CHILD",
        &[
            "acting",
            "first",
            "reload",
            "command-reload",
            "nested-reload",
        ],
    ) {
        return;
    }
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "observer",
        Box::new(Observer(observed.clone())),
    )]
    .into();
    mod_platform::__private::install(mods);
    let scratch = std::env::current_dir().unwrap().join("progress-data");
    let dir = scratch.join("data/testmod/advancement");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("progress.json"),
        r#"{
        "criteria": {
            "alpha": {"trigger": "minecraft:tick"},
            "beta": {"trigger": "minecraft:tick"},
            "gamma": {"trigger": "minecraft:tick"}
        },
        "requirements": [["alpha", "beta"], ["gamma"]],
        "rewards": {"recipes": ["testmod:reward"]}
    }"#,
    )
    .unwrap();
    let case = std::env::var("HORIZON_PROGRESS_SNAPSHOT_CHILD").unwrap();
    with_guests(|game| {
        if matches!(case.as_str(), "command-reload" | "nested-reload") {
            use crate::game::lan_actions::GuestCommand;
            game.world_root = scratch.join("world");
            let pack = game.world_root.join("datapacks/test");
            std::fs::create_dir_all(pack.join("data/testmod/function")).unwrap();
            std::fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"description":"test","pack_format":121}}"#,
            )
            .unwrap();
            std::fs::write(
                pack.join("data/testmod/function/reward.mcfunction"),
                "scoreboard players add @s reward_hits 1\n",
            )
            .unwrap();
            std::fs::write(
                game.world_root.join("datapacks.json"),
                r#"{"enabled":["file/test"],"disabled":[]}"#,
            )
            .unwrap();
            game.adv.tree = Arc::new(Tree::load(&scratch.join("data")));
            game.score.board.add_objective(
                "reward_hits",
                "dummy",
                serde_json::json!({"text":"reward_hits"}),
                horizon_client::scoreboard::RenderType::Integer,
                false,
                None,
            );
            game.lan.as_mut().unwrap().hub.guests.update_all(|guest| {
                guest.advancements_loaded = true;
                guest.advancements.grant(ID, "gamma", DATE);
            });
            game.lan.as_mut().unwrap().hub.mods_loaded = true;
            let reload = |game: &mut Game<'_>| {
                assert!(game.run_guest_command(GuestCommand {
                    conn: 12,
                    command: "reload".into(),
                    permission: 4,
                    integrated: true,
                    quiet: false,
                }));
            };
            if case == "nested-reload" {
                game.as_guest(13, reload).unwrap();
            } else {
                reload(game);
            }
            assert!(
                game.fns.store.functions.contains_key("testmod:reward"),
                "reward function was not loaded"
            );
            let events = observed.lock().unwrap();
            let first: Vec<_> = events.iter().filter(|event| event.player == 12).collect();
            assert_eq!(first.len(), 1);
            assert_eq!(first[0].criterion, "alpha");
            assert_eq!(first[0].kind, ProgressType::Grant);
            assert_eq!(
                first[0].rewarded,
                Some(false),
                "live state is read before rewards"
            );
            let progress = &first[0].progress;
            assert_eq!(
                progress
                    .criteria
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>(),
                ["alpha", "gamma"]
            );
            assert!(progress.criteria[0].obtained_at.is_some());
            assert_eq!(progress.criteria[1].obtained_at.as_deref(), Some(DATE));
            assert_eq!(progress.requirements, vec![vec!["alpha"], vec!["gamma"]]);
            assert_eq!(progress.completed_requirements, 2);
            assert!(progress.done);
            assert_eq!(events.len(), 2, "each guest receives its progress event");
            for event in events.iter() {
                if event.player == 12 {
                    assert_eq!(
                        event.progress.criteria[1].obtained_at.as_deref(),
                        Some(DATE)
                    );
                    assert!(event.progress.done);
                }
            }
            let kept = first[0].progress.clone();
            drop(events);
            for conn in [12, 13] {
                let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
                assert_eq!(guest.xp_total, if conn == 12 { 7 } else { 0 });
                {
                    let item = "minecraft:stone";
                    assert_eq!(
                        guest
                            .inventory
                            .iter()
                            .flatten()
                            .filter(|stack| stack.item.as_ref() == item)
                            .map(|stack| stack.count as usize)
                            .sum::<usize>(),
                        usize::from(conn == 12),
                        "{conn}: {item}"
                    );
                }
                if conn == 13 && case == "nested-reload" {
                    continue;
                }
                assert!(
                    guest.advancements.has(ID, "alpha"),
                    "alpha survives leaving the turn"
                );
                assert_eq!(guest.advancements.criteria[ID]["gamma"], DATE);
                assert!(
                    guest.recipe_book.knows("testmod:reward"),
                    "reward belongs to the same live player"
                );
            }
            assert_eq!(
                game.score
                    .board
                    .score("guest12", "reward_hits")
                    .map(|score| score.value),
                Some(1)
            );
            assert_eq!(
                game.score
                    .board
                    .score("guest13", "reward_hits")
                    .map(|score| score.value),
                None
            );
            if case == "nested-reload" {
                game.as_guest(13, |game| game.tick_advancements()).unwrap();
            }
            reload(game);
            assert_eq!(
                observed.lock().unwrap().len(),
                if case == "nested-reload" { 3 } else { 2 },
                "a repeated reload makes no duplicate grant"
            );
            for conn in [12, 13] {
                game.as_guest(conn, |game| game.tick_advancements())
                    .unwrap();
                let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
                assert_eq!(
                    guest.xp_total,
                    if conn == 12 || case == "nested-reload" {
                        7
                    } else {
                        0
                    }
                );
                {
                    let item = "minecraft:stone";
                    assert_eq!(
                        guest
                            .inventory
                            .iter()
                            .flatten()
                            .filter(|stack| stack.item.as_ref() == item)
                            .map(|stack| stack.count as usize)
                            .sum::<usize>(),
                        usize::from(conn == 12 || case == "nested-reload"),
                        "{conn}: {item}"
                    );
                }
            }
            assert_eq!(kept.criteria[1].obtained_at.as_deref(), Some(DATE));
            assert!(kept.done);
        } else if case == "acting" {
            game.adv.tree = Arc::new(Tree::load(&scratch.join("data")));
            // Another player has unrelated progress: the acting snapshot must not use it.
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| guest.advancements.grant(ID, "gamma", DATE))
                .unwrap();
            game.as_guest(12, |game| {
                game.adv.progress.grant(ID, "alpha", DATE);
                let index = game.adv.tree.by_id[ID];
                assert!(game.grant_advancement(index, Some("beta")));
                assert!(
                    !game.grant_advancement(index, Some("beta")),
                    "no duplicate event"
                );
                assert!(game.grant_advancement(index, Some("gamma")));
                assert!(game.revoke_advancement(index, Some("alpha")));
                assert!(game.revoke_advancement(index, Some("beta")));
                assert!(game.revoke_advancement(index, Some("gamma")));
                assert!(
                    !game.revoke_advancement(index, Some("gamma")),
                    "no no-op event"
                );
            })
            .unwrap();
            let events = observed.lock().unwrap();
            assert_eq!(events.len(), 5);
            assert!(events.iter().all(|event| event.player == 12));
            assert_eq!(
                events
                    .iter()
                    .map(|event| (event.criterion.as_str(), event.kind))
                    .collect::<Vec<_>>(),
                vec![
                    ("beta", ProgressType::Grant),
                    ("gamma", ProgressType::Grant),
                    ("alpha", ProgressType::Revoke),
                    ("beta", ProgressType::Revoke),
                    ("gamma", ProgressType::Revoke),
                ]
            );
            assert_progress(&events[0].progress, [true, true, false], 1, false);
            assert_progress(&events[1].progress, [true, true, true], 2, true);
            assert_progress(&events[2].progress, [false, true, true], 2, true);
            assert_progress(&events[3].progress, [false, false, true], 1, false);
            assert_progress(&events[4].progress, [false, false, false], 0, false);
            assert_eq!(
                events[0].progress.criteria[0].obtained_at.as_deref(),
                Some(DATE)
            );
            assert_eq!(
                events[0].progress.criteria[1].obtained_at,
                events[1].progress.criteria[1].obtained_at
            );
            assert_eq!(
                events[1].rewarded,
                Some(false),
                "completion is captured before rewards"
            );
        } else {
            let mut sync = horizon_client::lan::data::SyncData::empty();
            sync.advancements = horizon_client::lan::ServerAdvancements::load(
                &scratch,
                &horizon_world::loot::PackOverlay::NONE,
                // Rewards must name a recipe registered in the server's index.
                std::collections::BTreeMap::from([(
                    "testmod:reward".into(),
                    r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:diamond","count":1}}"#.into(),
                )]),
                Default::default(),
            );
            let reload = case == "reload";
            let host = game.lan.as_mut().unwrap();
            host.hub.guests.update_all(|guest| {
                guest.advancements_loaded = reload;
            });
            host.hub
                .update_guest(13, |guest| guest.advancements.grant(ID, "gamma", DATE))
                .unwrap();
            host.hub.mods_loaded = reload;
            game.with_advancement_hub(|hub, owners| hub.reload_resources(Arc::new(sync), owners));
            if reload {
                game.run_native_advancement_grants();
            } else {
                game.tick_lan(false);
            }
            let events = observed.lock().unwrap();
            assert_eq!(
                events.len(),
                4,
                "one mutation at a time, including each guest's prior progress"
            );
            assert!(
                events
                    .iter()
                    .all(|event| event.kind == ProgressType::Grant
                        && event.rewarded == Some(false))
            );
            let first: Vec<_> = events.iter().filter(|event| event.player == 12).collect();
            assert_eq!(
                first
                    .iter()
                    .map(|event| event.criterion.as_str())
                    .collect::<Vec<_>>(),
                ["alpha", "beta", "gamma"]
            );
            assert_progress(&first[0].progress, [true, false, false], 1, false);
            assert_progress(&first[1].progress, [true, true, false], 1, false);
            assert_progress(&first[2].progress, [true, true, true], 2, true);
            let second: Vec<_> = events.iter().filter(|event| event.player == 13).collect();
            assert_eq!(second.len(), 1);
            assert_eq!(second[0].criterion, "alpha");
            assert_progress(&second[0].progress, [true, false, true], 2, true);
            assert_eq!(
                second[0].progress.criteria[2].obtained_at.as_deref(),
                Some(DATE)
            );
            for conn in [12, 13] {
                assert!(
                    game.lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(conn)
                        .unwrap()
                        .recipe_book
                        .knows("testmod:reward")
                );
            }
        }
    });
    std::fs::remove_dir_all(scratch).unwrap();
}
