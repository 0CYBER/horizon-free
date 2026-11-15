//! The mouse-key portion of Minecraft.handleKeybinds. Events only count clicks;
//! the client tick consumes them before player movement and CLIENT_TICK_END.
use crate::input_queue::{ClickQueue, QueueLimits};

/// The bounds of each button's queued clicks, in [`Button`] order: a tick acts on 64 clicks of a
/// button and 1024 wait at most. A player clicks a few times a tick.
pub const CLICK_QUEUES: [QueueLimits; 3] = [
    QueueLimits {
        name: "attack clicks",
        max_len: 1024,
        per_tick: 64,
    },
    QueueLimits {
        name: "use clicks",
        max_len: 1024,
        per_tick: 64,
    },
    QueueLimits {
        name: "pick clicks",
        max_len: 1024,
        per_tick: 64,
    },
];

/// The empty click queue of `button`.
pub const fn click_queue(button: Button) -> ClickQueue {
    ClickQueue::new(CLICK_QUEUES[button as usize])
}

pub struct InteractionInput {
    clicks: [ClickQueue; 3],
}
impl Default for InteractionInput {
    fn default() -> Self {
        Self {
            clicks: [Button::Attack, Button::Use, Button::Pick].map(click_queue),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Button {
    Attack,
    Use,
    Pick,
}

pub trait InteractionWorld {
    /// Minecraft.pick before handleKeybinds: the tick's `hitResult`, which
    /// every click of the tick reads, even after one of them changed the
    /// block it names (a creative break, then a use that replaces it).
    fn pick_hit_result(&mut self);
    /// The tick's clicks are done: later picks follow the crosshair again.
    fn release_hit_result(&mut self);
    fn using_item(&self) -> bool;
    fn release_using(&mut self);
    fn start_attack(&mut self) -> bool;
    fn start_use(&mut self, held: bool);
    fn pick(&mut self);
    fn use_delay(&self) -> u32;
    fn continue_attack(&mut self, down: bool);
}

/// Minecraft.startUseItem dispatches one hit result per hand. A passing
/// entity interaction falls back to the item, without trying a block behind it.
pub trait TargetUseWorld {
    fn use_entity(&mut self) -> bool;
    fn use_block(&mut self) -> bool;
    fn use_item(&mut self);
}

pub fn use_on_target(world: &mut impl TargetUseWorld, entity: bool) {
    let consumed = if entity {
        world.use_entity()
    } else {
        world.use_block()
    };
    if !consumed {
        world.use_item();
    }
}

impl InteractionInput {
    pub fn press(&mut self, button: Button) {
        self.clicks[button as usize].press();
    }
    /// Take the pending input, leaving empty callback queues with the same refusal latches.
    /// The owner must merge callback input back after the tick. Clearing or temporarily taking
    /// input must not restart diagnostics during the lifetime of the owning Remote.
    pub fn take_queued(&mut self) -> Self {
        let empty = Self {
            clicks: self.clicks.map(|mut clicks| {
                clicks.clear();
                clicks
            }),
        };
        std::mem::replace(self, empty)
    }
    /// Preserve clicks queued by callbacks while the owner temporarily takes this input.
    pub fn merge_queued(&mut self, queued: Self) {
        for (clicks, added) in self.clicks.iter_mut().zip(queued.clicks) {
            clicks.merge(added);
        }
    }
    pub fn tick(&mut self, world: &mut impl InteractionWorld, attack_down: bool, use_down: bool) {
        world.pick_hit_result();
        let mut instant = false;
        if world.using_item() {
            if !use_down {
                world.release_using();
            }
            // Vanilla drains all three click queues while using an item,
            // including the tick that releases it.
            self.clicks.iter_mut().for_each(ClickQueue::clear);
        } else {
            // Up to each button's bound; the rest act on the next ticks (`CLICK_QUEUES`).
            let [attack, use_item, pick] = self.clicks.each_mut().map(|c| c.take_tick());
            for _ in 0..attack {
                instant |= world.start_attack();
            }
            for _ in 0..use_item {
                world.start_use(false);
            }
            for _ in 0..pick {
                world.pick();
            }
        }
        if use_down && world.use_delay() == 0 && !world.using_item() {
            world.start_use(true);
        }
        world.continue_attack(!instant && attack_down);
        world.release_hit_result();
    }
}

/// The sentence of `Minecraft.handleKeybinds` that follows a key which opened
/// a screen: `Gui.setScreen` has already run `KeyMapping.releaseAll()`, so
/// `key.use` reads up and the same call releases an active use
/// (`player.isUsingItem() && !keyUse.isDown()` → `MultiPlayerGameMode.releaseUsingItem`).
/// Only the screens opened from the keybinds do this (the inventory, chat and
/// advancements); the pause menu, server containers, mount inventories and
/// dialogs release on the first tick after closing instead, as vanilla does.
pub fn release_on_screen_open(world: &mut impl InteractionWorld) {
    if world.using_item() {
        world.release_using();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct World {
        using: bool,
        destroying: bool,
        delay: u32,
        begin_using: bool,
        events: Vec<&'static str>,
        use_attempts: Vec<bool>,
        callback_input: Option<InteractionInput>,
    }
    impl InteractionWorld for World {
        fn pick_hit_result(&mut self) {
            if let Some(input) = &mut self.callback_input {
                for _ in 0..1025 {
                    input.press(Button::Attack);
                }
            }
        }
        fn release_hit_result(&mut self) {}
        fn using_item(&self) -> bool {
            self.using
        }
        fn release_using(&mut self) {
            self.events.push("release");
            self.using = false;
        }
        fn start_attack(&mut self) -> bool {
            self.events.push("attack");
            false
        }
        fn start_use(&mut self, held: bool) {
            self.use_attempts.push(held);
            if !self.destroying {
                self.events.push("use");
                self.delay = 4;
                self.using = self.begin_using;
            }
        }
        fn pick(&mut self) {
            self.events.push("pick");
        }
        fn use_delay(&self) -> u32 {
            self.delay
        }
        fn continue_attack(&mut self, _: bool) {
            self.events.push("continue");
        }
    }

    /// Exercise the same take, callback, merge cycle as Remote without game resources.
    #[test]
    fn overflowing_callbacks_log_once_across_two_ticks() {
        struct Logger;
        thread_local! {
            static WARNINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        }
        impl log::Log for Logger {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                if record.level() == log::Level::Warn
                    && record.target() == "horizon_client_input::input_queue"
                {
                    WARNINGS.with(|count| count.set(count.get() + 1));
                }
            }
            fn flush(&self) {}
        }
        static LOGGER: Logger = Logger;
        log::set_logger(&LOGGER).expect("input tests own their logger");
        log::set_max_level(log::LevelFilter::Warn);
        let mut world = World {
            callback_input: Some(InteractionInput::default()),
            ..Default::default()
        };
        for tick in 0..2 {
            let mut input = world.callback_input.as_mut().unwrap().take_queued();
            world.events.clear();
            input.tick(&mut world, false, false);
            let queued = world.callback_input.take().unwrap();
            input.merge_queued(queued);
            world.callback_input = Some(input);
            assert_eq!(WARNINGS.with(std::cell::Cell::get), 1);
            assert_eq!(
                world
                    .events
                    .iter()
                    .filter(|&&event| event == "attack")
                    .count(),
                if tick == 0 { 0 } else { 64 }
            );
        }
        // Clearing on a screen transition must not restart diagnostics either.
        let mut discarded = world.callback_input.as_mut().unwrap().take_queued();
        discarded.tick(&mut world, false, false);
        assert_eq!(WARNINGS.with(std::cell::Cell::get), 1);
        // A new owner gets its own diagnostic, not a process-global latch.
        world.callback_input = Some(InteractionInput::default());
        InteractionInput::default().tick(&mut world, false, false);
        assert_eq!(WARNINGS.with(std::cell::Cell::get), 2);
    }

    #[test]
    fn quick_clicks_wait_for_a_tick_and_follow_keybind_order() {
        let mut input = InteractionInput::default();
        let mut world = World::default();
        input.press(Button::Use);
        input.press(Button::Pick);
        input.press(Button::Attack);
        assert!(world.events.is_empty());
        // Both edges can arrive between ticks; the press still acts once.
        input.tick(&mut world, false, false);
        assert_eq!(world.events, ["attack", "use", "pick", "continue"]);
        world.events.clear();
        input.tick(&mut world, false, false);
        assert_eq!(
            world.events,
            ["continue"],
            "no spurious release or repeated click"
        );
    }

    #[test]
    fn releasing_actual_item_use_discards_clicks_for_that_tick() {
        let mut input = InteractionInput::default();
        let mut world = World {
            using: true,
            ..Default::default()
        };
        for button in [Button::Attack, Button::Use, Button::Pick] {
            input.press(button);
        }
        input.tick(&mut world, true, false);
        assert_eq!(world.events, ["release", "continue"]);
    }

    #[test]
    fn holding_use_respects_the_delay_and_active_digging() {
        let mut input = InteractionInput::default();
        let mut world = World {
            destroying: true,
            ..Default::default()
        };
        input.press(Button::Use);
        input.tick(&mut world, true, true);
        assert_eq!(world.events, ["continue"]);
        world.destroying = false;
        world.events.clear();
        input.tick(&mut world, false, true);
        assert_eq!(world.events, ["use", "continue"]);
        world.events.clear();
        input.tick(&mut world, false, true);
        assert_eq!(world.events, ["continue"]);
        assert_eq!(world.use_attempts, [false, true, true]);
    }

    #[test]
    fn a_quick_use_click_releases_on_the_following_tick() {
        let mut input = InteractionInput::default();
        let mut world = World {
            begin_using: true,
            ..Default::default()
        };
        input.press(Button::Use);
        input.tick(&mut world, false, false);
        assert_eq!(world.events, ["use", "continue"]);
        assert!(world.using);
        world.events.clear();
        input.tick(&mut world, false, false);
        assert_eq!(world.events, ["release", "continue"]);
    }

    /// A floating block in front of the crosshair, nothing behind it, and a
    /// block item in the hand of a creative player.
    #[derive(Default)]
    struct Column {
        block: bool,
        hit: Option<Option<()>>,
        placed: bool,
    }
    impl Column {
        /// Minecraft.hitResult: the tick's pick, or a fresh one outside it.
        fn target(&self) -> Option<()> {
            self.hit.unwrap_or(self.block.then_some(()))
        }
    }
    impl InteractionWorld for Column {
        fn pick_hit_result(&mut self) {
            self.hit = Some(self.target());
        }
        fn release_hit_result(&mut self) {
            self.hit = None;
        }
        fn using_item(&self) -> bool {
            false
        }
        fn release_using(&mut self) {}
        fn start_attack(&mut self) -> bool {
            // MultiPlayerGameMode.startDestroyBlock in creative: the
            // predicted break leaves air.
            if self.target().is_some() {
                self.block = false;
            }
            !self.block
        }
        fn start_use(&mut self, _held: bool) {
            // BlockItem.place: the clicked block, now air, can be replaced.
            if self.target().is_some() && !self.block {
                self.block = true;
                self.placed = true;
            }
        }
        fn pick(&mut self) {}
        fn use_delay(&self) -> u32 {
            0
        }
        fn continue_attack(&mut self, _: bool) {}
    }

    #[test]
    fn attack_and_use_in_one_tick_share_the_hit_result() {
        // #3006: both buttons at once replace the block the crosshair is on.
        let mut input = InteractionInput::default();
        let mut world = Column {
            block: true,
            ..Default::default()
        };
        input.press(Button::Attack);
        input.press(Button::Use);
        input.tick(&mut world, false, false);
        assert!(world.placed, "the use acts on the block the attack broke");
        assert!(world.block);
        assert!(world.hit.is_none(), "the next pick follows the crosshair");
    }

    /// Each button's count stops at 1024 queued clicks.
    #[test]
    fn click_counts_stop_at_1024() {
        let mut input = InteractionInput::default();
        for _ in 0..1100 {
            for button in [Button::Attack, Button::Use, Button::Pick] {
                input.press(button);
            }
        }
        assert_eq!(input.clicks.map(|c| c.count()), [1024; 3]);
    }

    /// A player's burst acts in one tick in keybind order; a longer one acts 64 clicks of each
    /// button a tick, and the rest on the next ticks.
    #[test]
    fn a_long_burst_acts_64_clicks_of_each_button_a_tick() {
        let mut input = InteractionInput::default();
        let mut world = World::default();
        use Button::{Attack, Pick, Use};
        for button in [Attack, Attack, Attack, Use, Use, Pick] {
            input.press(button);
        }
        input.tick(&mut world, false, false);
        assert_eq!(
            world.events,
            [
                "attack", "attack", "attack", "use", "use", "pick", "continue"
            ]
        );
        for _ in 0..150 {
            input.press(Button::Attack);
        }
        for _ in 0..70 {
            input.press(Button::Use);
        }
        let mut ticks = Vec::new();
        for _ in 0..4 {
            world.events.clear();
            input.tick(&mut world, false, false);
            let count = |event| world.events.iter().filter(|e| **e == event).count();
            ticks.push([count("attack"), count("use")]);
        }
        assert_eq!(ticks, [[64, 64], [64, 6], [22, 0], [0, 0]]);
    }
}
