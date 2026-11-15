use super::*;
use crate::testing::FakeHost;

#[test]
fn chat_line_exposes_text_conversion_failure() {
    // This function must type-check for every client host, so callers can
    // propagate a refused text conversion instead of losing the error.
    fn add_line(host: &mut dyn ClientHost, line: Text) -> Result<(), TextFormError> {
        host.chat_line(line)
    }
    let _: fn(&mut dyn ClientHost, Text) -> Result<(), TextFormError> = add_line;
}

#[test]
#[ignore = "optimized hot-hook microbenchmark"]
fn subscriber_tick_benchmark() {
    use std::{hint::black_box, time::Instant};
    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {}
    struct Listener;
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn client_tick_pre(&self, _: &mut dyn ClientHost, event: &mut ClientTickPreEvent) {
            black_box(event);
        }
    }
    trait Before {
        fn client_tick_pre(&self, _: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {}
    }
    impl Before for Quiet {}
    impl Before for Listener {
        fn client_tick_pre(&self, _: &mut dyn ClientHost, event: &mut ClientTickPreEvent) {
            black_box(event);
        }
    }
    struct BeforeMod {
        id: &'static str,
        hooks: Box<dyn Before>,
    }
    let mut before: Vec<_> = (0..8)
        .map(|_| BeforeMod {
            id: "quiet",
            hooks: Box::new(Quiet),
        })
        .collect();
    before.push(BeforeMod {
        id: "listener",
        hooks: Box::new(Listener),
    });
    let mut installed: Vec<_> = (0..8)
        .map(|_| ModEntry::new("quiet", Box::new(Quiet)))
        .collect();
    installed.push(ModEntry::new("listener", Box::new(Listener)));
    install(installed.into());
    let mut host = FakeHost::new();
    dispatch_client_tick_pre(&mut host); // Warm the dispatch outside the timed loop.
    let iterations = 1_000_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let mut event = ClientTickPreEvent {};
        for entry in black_box(&before) {
            let _guard = mark_mod(entry.id, "client_tick_pre");
            entry.hooks.client_tick_pre(&mut host, &mut event);
        }
    }
    let baseline = start.elapsed().as_nanos() as f64 / f64::from(iterations);
    let start = Instant::now();
    for _ in 0..iterations {
        dispatch_client_tick_pre(black_box(&mut host));
    }
    let subscribed = start.elapsed().as_nanos() as f64 / f64::from(iterations);
    println!(
        "tick dispatch: before {baseline:.2} ns, subscribed {subscribed:.2} ns per call ({iterations} iterations)"
    );
}

#[test]
fn eager_subscribers_are_shared_across_dispatch_threads() {
    use std::sync::Barrier;
    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {}
    struct Listener;
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn loot_table_load(&self, _: &dyn DataLoadContext, _: &mut LootTableLoadEvent<'_>) {}
    }
    let installed = install(
        vec![
            ModEntry::new("quiet", Box::new(Quiet)),
            ModEntry::new("listener", Box::new(Listener)),
        ]
        .into(),
    );
    assert!(
        !installed
            .subscribers
            .contains_any(HookId::BabyEntitySpawn as usize)
    );
    assert_eq!(
        installed
            .subscribers
            .indices(HookId::LootTableLoad as usize)
            .collect::<Vec<_>>(),
        [1]
    );
    let barrier = Barrier::new(2);
    let load = crate::world::phase::TestReader::default();
    let context = crate::OwningLoad::new(crate::DataLoadOrigin::ServerDataLoad, &load);
    let context = &context;
    std::thread::scope(|scope| {
        for _ in 0..2 {
            let barrier = &barrier;
            scope.spawn(move || {
                TEST_MODS.set(Some(installed));
                barrier.wait();
                dispatch_loot_table_load(context, "test", &mut serde_json::json!([]));
            });
        }
    });
    assert_eq!(
        installed
            .subscribers
            .indices(HookId::LootTableLoad as usize)
            .collect::<Vec<_>>(),
        [1]
    );
}

#[test]
fn scoped_mods_dispatch_and_restore_after_panic() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use testing::with_dispatch;

    struct Counter(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Counter {
        fn client_tick_pre(&self, _: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    let outer = Arc::new(AtomicUsize::new(0));
    let inner = Arc::new(AtomicUsize::new(0));
    with_dispatch(
        vec![ModEntry::new(
            "outer-counter",
            Box::new(Counter(outer.clone())),
        )]
        .into(),
        || {
            dispatch_client_tick_pre(&mut FakeHost::new());
            let result = std::panic::catch_unwind(|| {
                with_dispatch(
                    vec![ModEntry::new(
                        "inner-counter",
                        Box::new(Counter(inner.clone())),
                    )]
                    .into(),
                    || {
                        dispatch_client_tick_pre(&mut FakeHost::new());
                        panic!("test unwind");
                    },
                );
            });
            assert!(result.is_err());
            dispatch_client_tick_pre(&mut FakeHost::new());
        },
    );
    assert_eq!(outer.load(Ordering::Relaxed), 2);
    assert_eq!(inner.load(Ordering::Relaxed), 1);
    assert!(mods().is_empty());
}

#[test]
fn native_switch_skips_and_resumes_subscribed_hooks() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counter(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Counter {
        fn client_tick_pre(&self, _: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let count = Arc::new(AtomicUsize::new(0));
    testing::with_dispatch(
        vec![ModEntry::new("counter", Box::new(Counter(count.clone())))].into(),
        || {
            assert_eq!(__private::mod_index("counter"), Some(0));
            dispatch_client_tick_pre(&mut FakeHost::new());
            assert!(__private::set_mod_enabled("counter", false));
            dispatch_client_tick_pre(&mut FakeHost::new());
            assert!(__private::set_mod_enabled("counter", true));
            dispatch_client_tick_pre(&mut FakeHost::new());
            assert!(!__private::set_mod_enabled("missing", false));
        },
    );
    assert_eq!(count.load(Ordering::Relaxed), 2);
}

#[test]
fn movement_input_writes_back_all_fields() {
    struct Movement;
    #[mod_api::__subscriptions]
    impl Hooks for Movement {
        fn movement_input(&self, _host: &dyn ClientHost, event: &mut MovementInputEvent) {
            event.forward = -0.5;
            event.strafe = 0.25;
            event.jump = true;
            event.sneak = true;
            assert!(!event.forward_key);
            assert!(event.backward_key);
            assert!(!event.left_key);
            assert!(event.right_key);
            assert!(!event.sprint);
            event.forward_key = true;
            event.backward_key = false;
            event.left_key = true;
            event.right_key = false;
            event.sprint = true;
        }
    }
    let mods = [ModEntry::new("movement", Box::new(Movement))];
    install(mods.into());
    let (mut forward, mut strafe, mut jump, mut sneak) = (1.0, 0.0, false, false);
    let (mut forward_key, mut backward_key, mut left_key, mut right_key, mut sprint) =
        (false, true, false, true, false);
    dispatch_movement_input(
        &FakeHost::new(),
        crate::__private::MovementInputCall {
            forward: &mut forward,
            strafe: &mut strafe,
            jump: &mut jump,
            sneak: &mut sneak,
            forward_key: &mut forward_key,
            backward_key: &mut backward_key,
            left_key: &mut left_key,
            right_key: &mut right_key,
            sprint: &mut sprint,
        },
    );
    assert_eq!((forward, strafe, jump, sneak), (-0.5, 0.25, true, true));
    assert_eq!(
        (forward_key, backward_key, left_key, right_key, sprint),
        (true, false, true, false, true),
    );
}

#[test]
fn recipe_book_categories_accept_mod_declarations() {
    use mod_api::{RecipeBookCategoriesEvent, RecipeBookCategory, dispatch_recipe_book_categories};
    struct Categories;
    #[mod_api::__subscriptions]
    impl Hooks for Categories {
        fn recipe_book_categories(
            &self,
            _host: crate::HostSide,
            event: &mut RecipeBookCategoriesEvent<'_>,
        ) {
            event.categories.push(RecipeBookCategory {
                id: "mymod:crushing".into(),
                recipe_type: "mymod:crusher".into(),
                icon: "minecraft:stone".into(),
                target_menu: "mymod:crusher".into(),
                book: crate::item::recipe_book::RecipeBookTypeKey {
                    id: "mymod:crusher".into(),
                },
                search_group: "mymod:search".into(),
                displays: Vec::new(),
            });
        }
    }
    let mut declarations = Vec::new();
    dispatch_recipe_book_categories(HostSide::Server(&mut FakeHost::new()), &mut declarations);
    assert!(declarations.is_empty());
    let mods = [ModEntry::new("categories", Box::new(Categories))];
    install(mods.into());
    dispatch_recipe_book_categories(HostSide::Server(&mut FakeHost::new()), &mut declarations);
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].id, "mymod:crushing");
    assert_eq!(declarations[0].recipe_type, "mymod:crusher");
    assert_eq!(declarations[0].icon, "minecraft:stone");
}

#[test]
fn client_tick_pre_and_post_hear_their_own_moment() {
    use std::sync::Mutex;
    struct Ticks(Arc<Mutex<Vec<&'static str>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Ticks {
        fn client_tick_pre(&self, _: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
            self.0.lock().unwrap().push("pre");
        }
        fn client_tick_post(&self, _: &mut dyn ClientHost, _: &mut ClientTickPostEvent) {
            self.0.lock().unwrap().push("post");
        }
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    testing::with_dispatch(
        vec![ModEntry::new("ticks", Box::new(Ticks(heard.clone())))].into(),
        || {
            dispatch_client_tick_pre(&mut FakeHost::new());
            dispatch_client_tick_post(&mut FakeHost::new());
        },
    );
    assert_eq!(*heard.lock().unwrap(), ["pre", "post"]);
}

#[test]
fn level_tick_pre_and_post_hear_the_level_in_their_own_moment() {
    use std::sync::Mutex;
    struct Ticks(Arc<Mutex<Vec<(&'static str, LevelRef)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Ticks {
        fn level_tick_pre(&self, _: HostSide, event: &mut LevelTickPreEvent) {
            self.0.lock().unwrap().push(("pre", event.level));
        }
        fn level_tick_post(&self, _: HostSide, event: &mut LevelTickPostEvent) {
            self.0.lock().unwrap().push(("post", event.level));
        }
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    let level = LevelRef::new(WorldEpoch::new(4), DimensionKey::OVERWORLD);
    testing::with_dispatch(
        vec![ModEntry::new("ticks", Box::new(Ticks(heard.clone())))].into(),
        || {
            dispatch_level_tick_pre(HostSide::Client(&mut FakeHost::new()), level);
            dispatch_level_tick_post(HostSide::Client(&mut FakeHost::new()), level);
        },
    );
    assert_eq!(*heard.lock().unwrap(), [("pre", level), ("post", level)]);
}

#[test]
fn living_death_post_hears_the_entity_the_blow_and_the_attacker() {
    use std::sync::Mutex;
    struct Post(Arc<Mutex<Vec<(u32, String, u32)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Post {
        fn living_death_post(&self, _: &mut dyn ServerHost, event: &mut LivingDeathPostEvent<'_>) {
            self.0.lock().unwrap().push((
                event.entity,
                event.damage_type.to_owned(),
                event.attacker,
            ));
        }
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    testing::with_dispatch(
        vec![ModEntry::new("post", Box::new(Post(heard.clone())))].into(),
        || dispatch_living_death_post(&mut FakeHost::new(), 7, "minecraft:fall", 0),
    );
    assert_eq!(
        *heard.lock().unwrap(),
        [(7, "minecraft:fall".to_owned(), 0)]
    );
}

#[test]
fn render_frame_pre_and_post_hear_their_own_moment_with_the_frames_time() {
    use std::{sync::Mutex, time::Duration};
    type FrameReading = (&'static str, Duration, f32, f32);
    struct Frames(Arc<Mutex<Vec<FrameReading>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Frames {
        fn render_frame_pre(&self, _: &mut dyn ClientHost, event: &mut RenderFramePreEvent) {
            self.0.lock().unwrap().push((
                "pre",
                event.elapsed,
                event.world_partial_tick,
                event.partial_tick,
            ));
        }
        fn render_frame_post(&self, _: &mut dyn ClientHost, event: &mut RenderFramePostEvent) {
            self.0.lock().unwrap().push((
                "post",
                event.elapsed,
                event.world_partial_tick,
                event.partial_tick,
            ));
        }
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    testing::with_dispatch(
        vec![ModEntry::new("frames", Box::new(Frames(heard.clone())))].into(),
        || {
            let frame = Duration::from_millis(16);
            dispatch_render_frame_pre(&mut FakeHost::new(), frame, 0.25, 0.5);
            dispatch_render_frame_post(&mut FakeHost::new(), frame, 0.25, 0.5);
        },
    );
    let frame = Duration::from_millis(16);
    assert_eq!(
        *heard.lock().unwrap(),
        [("pre", frame, 0.25, 0.5), ("post", frame, 0.25, 0.5)]
    );
}

#[test]
fn render_gui_pre_cancel_reaches_the_game() {
    use std::sync::Mutex;
    struct Gui(Arc<Mutex<Vec<&'static str>>>, bool);
    #[mod_api::__subscriptions]
    impl Hooks for Gui {
        fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
            self.0.lock().unwrap().push("pre");
            if self.1 { Flow::Cancel } else { Flow::Continue }
        }
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mut canceled = false;
    testing::with_dispatch(
        vec![ModEntry::new("gui", Box::new(Gui(heard.clone(), true)))].into(),
        || dispatch_render_gui_pre(&FakeHost::new(), &mut canceled),
    );
    assert!(canceled, "a mod's cancel reaches the game");
    assert_eq!(*heard.lock().unwrap(), ["pre"]);
}

/// What the GUI listeners below heard, in order: each mod's name and whether it saw a cancel.
type Heard = Arc<std::sync::Mutex<Vec<(&'static str, bool)>>>;

/// A `render_gui_pre` listener of each kind `#[listener]` gives: it records the event and answers
/// `answer`; `Uncancels` clears a cancel it receives.
#[derive(Clone, Copy)]
enum Answer {
    Continues,
    Cancels,
    Uncancels,
}

/// Dispatches `render_gui_pre` to `mods` (name, listener) in that dependency order, with the
/// event cancelled up front when `cancelled`; returns what reached the game and who heard what.
fn gui_dispatch(
    cancelled: bool,
    mods: Vec<(&'static str, Box<dyn Fn(Heard) -> Box<dyn Hooks>>)>,
) -> (bool, Vec<(&'static str, bool)>) {
    let heard = Heard::default();
    let mods: Vec<_> = mods
        .into_iter()
        .map(|(id, make)| ModEntry::new(id, make(heard.clone())))
        .collect();
    let mut cancelled = cancelled;
    testing::with_dispatch(mods.into(), || {
        dispatch_render_gui_pre(&FakeHost::new(), &mut cancelled)
    });
    let heard = heard.lock().unwrap().clone();
    (cancelled, heard)
}

fn answer(
    heard: &Heard,
    name: &'static str,
    answer: Answer,
    event: &mut RenderGuiPreEvent,
) -> Flow {
    heard.lock().unwrap().push((name, event.cancelled));
    match answer {
        Answer::Continues => Flow::Continue,
        Answer::Cancels => Flow::Cancel,
        Answer::Uncancels => {
            event.cancelled = false;
            Flow::Continue
        }
    }
}

/// A listener at the default priority, not told of cancelled events.
fn plain(name: &'static str, kind: Answer) -> (&'static str, Box<dyn Fn(Heard) -> Box<dyn Hooks>>) {
    struct Plain(Heard, &'static str, Answer);
    #[mod_api::__subscriptions]
    impl Hooks for Plain {
        fn render_gui_pre(&self, _: &dyn ClientHost, event: &mut RenderGuiPreEvent) -> Flow {
            answer(&self.0, self.1, self.2, event)
        }
    }
    (
        name,
        Box::new(move |heard| Box::new(Plain(heard, name, kind))),
    )
}

#[test]
fn listeners_run_by_priority_then_dependency_order() {
    struct First(Heard);
    #[mod_api::__subscriptions]
    impl Hooks for First {
        #[listener(priority = 10)]
        fn render_gui_pre(&self, _: &dyn ClientHost, event: &mut RenderGuiPreEvent) -> Flow {
            answer(&self.0, "first", Answer::Continues, event)
        }
    }
    struct AlsoFirst(Heard);
    #[mod_api::__subscriptions]
    impl Hooks for AlsoFirst {
        #[listener(priority = 10)]
        fn render_gui_pre(&self, _: &dyn ClientHost, event: &mut RenderGuiPreEvent) -> Flow {
            answer(&self.0, "also_first", Answer::Continues, event)
        }
    }
    struct Last(Heard);
    #[mod_api::__subscriptions]
    impl Hooks for Last {
        #[listener(priority = -5)]
        fn render_gui_pre(&self, _: &dyn ClientHost, event: &mut RenderGuiPreEvent) -> Flow {
            answer(&self.0, "last", Answer::Continues, event)
        }
    }
    let (cancelled, heard) = gui_dispatch(
        false,
        vec![
            plain("plain", Answer::Continues),
            ("last", Box::new(|heard| Box::new(Last(heard)))),
            ("first", Box::new(|heard| Box::new(First(heard)))),
            ("also_first", Box::new(|heard| Box::new(AlsoFirst(heard)))),
        ],
    );
    assert!(!cancelled);
    let order: Vec<_> = heard.iter().map(|(name, _)| *name).collect();
    // Equal priorities keep dependency order: `first` was installed before `also_first`.
    assert_eq!(order, ["first", "also_first", "plain", "last"]);
}

/// Receives cancelled events at the default priority, answering `kind`.
struct Receiver(Heard, &'static str, Answer);
#[mod_api::__subscriptions]
impl Hooks for Receiver {
    #[listener(receive_cancelled)]
    fn render_gui_pre(&self, _: &dyn ClientHost, event: &mut RenderGuiPreEvent) -> Flow {
        answer(&self.0, self.1, self.2, event)
    }
}

fn receiver(
    name: &'static str,
    kind: Answer,
) -> (&'static str, Box<dyn Fn(Heard) -> Box<dyn Hooks>>) {
    (
        name,
        Box::new(move |heard| Box::new(Receiver(heard, name, kind))),
    )
}

#[test]
fn a_cancel_skips_later_listeners_except_those_receiving_cancelled_events() {
    let (cancelled, heard) = gui_dispatch(
        false,
        vec![
            plain("canceller", Answer::Cancels),
            plain("skipped", Answer::Continues),
            receiver("watcher", Answer::Continues),
        ],
    );
    assert!(cancelled, "the cancel reaches the game");
    assert_eq!(heard, [("canceller", false), ("watcher", true)]);
}

#[test]
fn a_listener_receiving_a_cancel_can_undo_it() {
    let (cancelled, heard) = gui_dispatch(
        false,
        vec![
            plain("canceller", Answer::Cancels),
            receiver("undoer", Answer::Uncancels),
            plain("after", Answer::Continues),
        ],
    );
    assert!(!cancelled, "the undone cancel does not reach the game");
    // Once the cancel is undone, the listeners after it are told again.
    assert_eq!(
        heard,
        [("canceller", false), ("undoer", true), ("after", false)]
    );
}

#[test]
fn an_event_cancelled_before_dispatch_reaches_only_listeners_receiving_cancelled_events() {
    let (cancelled, heard) = gui_dispatch(true, vec![plain("plain", Answer::Continues)]);
    assert!(cancelled);
    assert!(heard.is_empty(), "no listener receives cancelled events");
    let (cancelled, heard) = gui_dispatch(
        true,
        vec![
            plain("plain", Answer::Continues),
            receiver("undoer", Answer::Uncancels),
        ],
    );
    assert!(!cancelled);
    assert_eq!(heard, [("undoer", true)]);
}

#[test]
fn window_resize_delivers_physical_and_gui_dimensions() {
    use std::sync::Mutex;
    struct Listener(Arc<Mutex<Vec<(u32, u32, u32, u32)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn window_resize(&self, _: &mut dyn ClientHost, event: &mut WindowResizeEvent) {
            self.0.lock().unwrap().push((
                event.width,
                event.height,
                event.gui_width,
                event.gui_height,
            ));
        }
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    testing::with_dispatch(
        vec![ModEntry::new("resize", Box::new(Listener(heard.clone())))].into(),
        || {
            dispatch_window_resize(&mut FakeHost::new(), 1200, 900, 400, 300);
        },
    );
    assert_eq!(*heard.lock().unwrap(), [(1200, 900, 400, 300)]);
}

/// Compile-only: a server mod reaches the server's inventory and menu operations through the
/// `&mut dyn ServerHost` its hooks get, not only through the concrete game.
#[allow(dead_code)]
fn server_inventory_operations_take_a_server_host(
    host: &mut dyn ServerHost,
    player: PlayerRef,
    menu: MenuRef,
    stack: ItemStack,
) -> (bool, Result<Option<ItemStack>, StackWriteError>, bool) {
    (
        player.set_selected_slot(host, 0),
        player.drop_stack(host, stack, false),
        menu.close_container(host),
    )
}

/// Compile-only: a server mod schedules and cancels a delayed task through the `&mut dyn ServerHost`
/// its hooks get.
#[allow(dead_code)]
fn delayed_tasks_take_a_server_host(
    host: &mut dyn ServerHost,
) -> (Result<TaskId, ScheduleError>, bool) {
    (host.schedule_task(20, 7), host.cancel_task(TaskId(1)))
}

/// Compile-only: a mod reads a level's dimension type, and registers a dimension with its own.
#[allow(dead_code)]
fn dimension_types_take_a_host(
    host: &dyn Host,
    level: LevelRef,
) -> Option<(f64, bool, DimensionSettings)> {
    let ty = level.dimension_type(host)?;
    let nether_like = DimensionSettings {
        coordinate_scale: 8.0,
        has_fixed_time: true,
        fast_lava: true,
        ..Default::default()
    };
    Some((ty.coordinate_scale, ty.fast_lava, nether_like))
}

/// Compile-only: a mod reads the time of a level's own clock from a handle on a shared host.
#[allow(dead_code)]
fn clock_time_takes_a_host(host: &dyn Host, level: LevelRef) -> Option<i64> {
    level.clock_time(host)
}

/// Compile-only: a mod reads when a level's own clock next reaches a time marker.
#[allow(dead_code)]
fn clock_marker_time_takes_a_host(
    host: &dyn Host,
    level: LevelRef,
    marker: &crate::Identifier,
) -> Option<i64> {
    level.clock_marker_time(host, marker)
}

/// Compile-only: a mod reads a level's sea level from a handle on a shared host.
#[allow(dead_code)]
fn sea_level_takes_a_host(host: &dyn Host, level: LevelRef) -> Option<i32> {
    level.sea_level(host)
}

/// Compile-only: a client mod reads column heights of the world its window shows.
#[allow(dead_code)]
fn client_heights_take_a_client_host(host: &dyn ClientHost) -> Result<Vec<Option<i32>>, String> {
    host.client_surface_heights(&[(0, 0), (16, -16)], Heightmap::MotionBlocking)
}

/// Compile-only: a client mod moves the view onto an entity and back through the `&mut dyn ClientHost`
/// its hooks get.
#[allow(dead_code)]
fn camera_entity_takes_a_client_host(host: &mut dyn ClientHost, mob: EntityRef) -> bool {
    let moved = host.set_camera_entity(mob);
    let own = host.camera_entity();
    moved && own.is_some()
}

/// Compile-only: a client mod reads the open inventory screen's slot boxes through its menu handle.
#[allow(dead_code)]
fn menu_slot_boxes_take_a_client_host(
    host: &dyn ClientHost,
    menu: MenuRef,
) -> Option<Vec<Option<[f32; 4]>>> {
    menu.slot_boxes(host)
}

/// Compile-only: the world data, tick, sky and sight queries as a mod calls them, the reads on any
/// host and the writes (which move the level's random or its saved data) on a server host.
#[allow(dead_code)]
fn world_data_and_tick_queries_take_their_hosts(
    read: &dyn Host,
    server: &mut dyn ServerHost,
    level: LevelRef,
) {
    let _: Option<RespawnData> = level.respawn_data(read);
    let _: Option<MapState> = level.map_data(read, 0);
    let _: Option<bool> = level.can_see_sky(read, [0, 64, 0]);
    let _: Option<bool> = level.line_of_sight(read, [0.0, 65.0, 0.0], [8.0, 65.0, 0.0]);
    let _: bool = level.set_respawn_data(server, [0, 64, 0], 90.0, 0.0);
    let _: bool = level.set_map_data(server, 0, &[0; 128 * 128]);
    let _: bool = level.block_tick(server, [0, 64, 0]);
    let _: bool = level.block_random_tick(server, [0, 64, 0]);
    let _: Option<[i32; 3]> = level.lightning_target(server, [0, 64, 0]);
    let _: bool = level.has_water_ticket(read, [0, 64, 0]);
    if let Some(ticket) = level.add_water_ticket(server, [0, 64, 0], [5, 65, 5]) {
        let _: bool = ticket.remove(server);
    }
}

/// Compile-only: the level queries as a mod calls them, the reads on any host and the drop
/// roll, which moves the level's random, on a server host.
#[allow(dead_code)]
fn level_queries_take_their_hosts(
    read: &dyn Host,
    server: &mut dyn ServerHost,
    level: LevelRef,
    chunk: ChunkRef,
    shooter: EntityRef,
    tool: &ItemStack,
) {
    let _: Option<Vec<PlayerRef>> = level.online_players(read);
    let _: Option<Vec<PlayerRef>> = chunk.watchers(read);
    let _: Option<StructureSearchResult> = level.find_nearest_structure(
        read,
        "#minecraft:village",
        StructureSearch::new([0, 64, 0], 100),
    );
    let _: Option<Vec<[f64; 6]>> = level.fluid_shape(read, [0, 64, 0]);
    let _: Option<i32> = level.fluid_tick_delay(read, FluidKey(0), [0, 64, 0]);
    let _: Option<f32> = level.fluid_height(read, [0, 64, 0]);
    let _: Option<f32> = level.fluid_height_for_camera(read, [0, 64, 0]);
    let _: Option<bool> =
        level.fluid_can_be_replaced_with(read, [0, 64, 0], FluidKey(0), Direction::Down);
    let _: Option<bool> = level.has_collisions(read, [0.0, 64.0, 0.0, 1.0, 65.0, 1.0]);
    let _: Option<Vec<EntityRayHit>> =
        level.ray_trace_entities(read, [0.0; 3], [8.0, 0.0, 0.0], 0.3, Some(shooter));
    let _: Result<Option<Vec<ItemStack>>, StackWriteError> =
        level.block_drops(server, [0, 64, 0], Some(tool), Some(shooter));
    let _: Result<Option<Vec<ItemStack>>, StackWriteError> = level.roll_loot(
        server,
        "mymod:sifting/gravel",
        [0.5, 65.0, 0.5],
        Some(tool),
        Some(shooter),
    );
    let _: bool = level.fluid_tick(server, [0, 64, 0]);
}

/// Compile-only: the entity queries for contact, picking and multipart parts as a mod calls
/// them, all reads on a shared host.
#[allow(dead_code)]
fn entity_queries_take_a_shared_host(read: &dyn Host, entity: EntityRef) {
    let _: Option<[f64; 3]> = entity.up_vector(read);
    let _: Option<[i32; 3]> = entity.on_block(read);
    let _: Option<ClipHit> = entity.pick(read, 4.5);
    let _: Option<bool> = entity.is_multipart(read);
    let _: Option<Vec<EntityRef>> = entity.parts(read);
}

/// Compile-only: an entity is sent to another dimension from a server host.
#[allow(dead_code)]
fn entity_travel_takes_a_server_host(server: &mut dyn ServerHost, entity: EntityRef) {
    let _: bool = entity.teleport_to_level(server, "mymod:pocket", [8.5, 100.0, 8.5], None);
    let _: bool = entity.teleport_to_level(
        server,
        "minecraft:the_nether",
        [0.0, 64.0, 0.0],
        Some([90.0, 0.0]),
    );
}

/// Compile-only: a level is asked to be saved from a server host.
#[allow(dead_code)]
fn level_save_takes_a_server_host(server: &mut dyn ServerHost, level: LevelRef) -> bool {
    level.request_save(server)
}

/// Compile-only: a block event is queued from a server host.
#[allow(dead_code)]
fn block_event_takes_a_server_host(
    server: &mut dyn ServerHost,
    level: LevelRef,
    block: BlockKey,
) -> bool {
    level.block_event(server, [0, 64, 0], block, 0, 0)
}

/// Compile-only: the player rule reads on any host, and the writes on a server host.
#[allow(dead_code)]
fn player_rules_take_their_hosts(
    read: &dyn Host,
    server: &mut dyn ServerHost,
    player: PlayerRef,
    other: PlayerRef,
    stack: &ItemStack,
    config: PlayerRespawnConfig,
) {
    let _: Option<bool> = player.may_use_item_at(read, [0, 64, 0], BlockFace::Up, stack);
    let _: Option<bool> = player.can_harm(read, other);
    let _: Option<Option<PlayerRespawnConfig>> = player.respawn_config(read);
    let _: Option<bool> = player.text_filtering(read);
    let _: Option<bool> = player.sleep_ignored(read);
    let _: Option<bool> = player.counts_for_sleep(read);
    let _: Option<u32> = player.sleep_timer(read);
    let _: bool = player.set_respawn_config(server, Some(config));
    let _: bool = player.set_sleep_timer(server, 0);
}

#[test]
fn the_special_multiplier_rises_between_effective_two_and_four() {
    let at = |effective| Difficulty::new(3, effective).special_multiplier();
    assert_eq!(at(1.5), 0.0);
    assert_eq!(at(3.0), 0.5);
    assert_eq!(at(4.5), 1.0);
}

/// Compile-only: the border, weather and difficulty calls take their hosts.
#[allow(dead_code)]
fn level_world_controls_take_a_server_host(server: &mut dyn ServerHost, level: LevelRef) {
    let _ = level.local_difficulty(server, [0, 64, 0]);
    let _ = level.set_world_border_center(server, [0.0, 0.0]);
    let _ = level.set_world_border_size(server, 100.0, 20);
    let _ = level.set_world_border_damage(server, 0.2, 5.0);
    let _ = level.set_world_border_warning(server, 5, 300);
    let _ = level.set_weather(server, 0, 6000, true, false);
    let _ = level.set_weather_levels(server, 1.0, 0.0);
    let _ = server.set_tick_rate(40.0);
}

/// Compile-only: mob spawning and the spawn policy as a mod calls them, the reads on any host
/// and the writes, which change what the level spawns, on a server host.
#[allow(dead_code)]
fn spawn_policy_writes_take_a_server_host(
    read: &dyn Host,
    server: &mut dyn ServerHost,
    level: LevelRef,
) {
    let _: Option<SpawnLimits> = level.spawn_limits(read);
    let _: Option<SpawnFlags> = level.spawn_flags(read);
    let _: Option<bool> = level.can_spawn_entities_in_chunk(read, [0, 0]);
    let _: bool = level.set_spawn_limits(server, SpawnLimits::VANILLA);
    let _: bool = level.set_spawn_flags(server, SpawnFlags::default());
    let _: Option<EntityRef> = level.spawn_mob(
        server,
        "minecraft:zombie",
        [0.5, 64.0, 0.5],
        SpawnReason::Command,
    );
}

/// Compile-only: a passage is registered and removed through its level on a server host.
#[allow(dead_code)]
fn passages_take_a_server_host(server: &mut dyn ServerHost, level: LevelRef, passage: Passage) {
    let id: Result<PassageId, PassageRegistrationError> = level.register_passage(server, passage);
    let _: bool = level.remove_passage(server, id.unwrap());
}
