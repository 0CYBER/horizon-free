use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    pub(in crate::game) fn frame_partial_ticks(&self) -> (f32, f32) {
        let partial_tick = self.clock.window_fraction();
        let world_partial_tick = if !self.shows_panorama() && self.player.is_some() {
            self.clock.fraction() as f32
        } else {
            partial_tick
        };
        (world_partial_tick, partial_tick)
    }

    pub(in crate::game) fn frame_world_partial_tick(&self) -> f64 {
        self.extras
            .frame_partial_ticks
            .map_or_else(|| self.clock.fraction(), |(world, _)| f64::from(world))
    }

    /// Once a frame, in a world or not, after Horizon's extras and just
    /// before the frame is rendered (`RenderFrameEvent.Pre`).
    pub(in crate::game) fn native_render_frame_pre(
        &mut self,
        elapsed: Duration,
        world_partial_tick: f32,
        partial_tick: f32,
    ) {
        self.fire_render_frame_pre(elapsed, world_partial_tick, partial_tick);
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
    use super::super::particle_count_tests::frame_scene;
    use horizon_client::{clock::FixedStep, title::Title};
    use mod_api::{
        ClientHost, Hooks, ModEntry, RenderFramePostEvent, RenderFramePreEvent,
        RenderLevelStageEvent,
    };
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    #[derive(Default)]
    struct Heard {
        pre: Vec<(f32, f32)>,
        post: Vec<(f32, f32)>,
        stages: Vec<f32>,
    }
    struct Frames(Arc<Mutex<Heard>>);
    #[mod_api::__subscriptions]
    impl Hooks for Frames {
        fn render_frame_pre(&self, _: &mut dyn ClientHost, event: &mut RenderFramePreEvent) {
            self.0
                .lock()
                .unwrap()
                .pre
                .push((event.world_partial_tick, event.partial_tick));
            // Listener edits must not become the renderer's or post event's time.
            event.world_partial_tick = -1.0;
            event.partial_tick = -1.0;
        }
        fn render_frame_post(&self, _: &mut dyn ClientHost, event: &mut RenderFramePostEvent) {
            self.0
                .lock()
                .unwrap()
                .post
                .push((event.world_partial_tick, event.partial_tick));
        }
        fn render_level_stage(&self, _: &dyn ClientHost, event: &mut RenderLevelStageEvent<'_>) {
            self.0.lock().unwrap().stages.push(event.view.partial_tick);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn frame_interpolation_matches_draw_and_holds_while_paused_or_frozen() {
        let name = "game::native_hooks::render_frame_pre::tests::frame_interpolation_matches_draw_and_holds_while_paused_or_frozen";
        if run_fresh_cases(name, "HORIZON_FRAME_PARTIAL_CHILD", &["frames"]) {
            return;
        }
        let heard = Arc::new(Mutex::new(Heard::default()));
        let mods = vec![ModEntry::new("frames", Box::new(Frames(heard.clone())))];
        mod_platform::testing::with_mods(mods.into(), || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                game.clock = FixedStep::client();
                game.clock.set_frozen(false);
                game.clock.end_frame(false);
                let elapsed = Duration::from_secs_f64(1.0 / 60.0);
                let frame = |game: &mut super::Game<'_>, world: bool| {
                    game.clock.advance(elapsed, 50.0);
                    game.extras_frame(elapsed);
                    // A clock change after pre cannot change the captured draw/post value.
                    game.clock.advance(Duration::from_millis(1), 50.0);
                    game.render_and_flip(elapsed).unwrap();
                    let mut h = heard.lock().unwrap();
                    let pre = *h.pre.last().unwrap();
                    assert_eq!(h.post.last(), Some(&pre));
                    if world {
                        assert!(!h.stages.is_empty());
                        assert!(h.stages.iter().all(|&partial| partial == pre.0));
                    } else {
                        assert!(h.stages.is_empty());
                        assert_eq!(pre.0, pre.1);
                    }
                    h.stages.clear();
                    pre
                };
                let running = frame(game, true);
                assert_eq!(running, (0.32, 0.32));
                let held = game.clock.fraction() as f32;
                game.clock.end_frame(true);
                let paused = frame(game, true);
                let paused_next = frame(game, true);
                assert_eq!(paused.0, held);
                assert_eq!(paused_next.0, held);
                assert_ne!(paused.1, paused_next.1);
                game.clock.set_frozen(true);
                assert!(!game.clock.world_runs());
                game.clock.end_frame(false);
                let frozen = frame(game, true);
                let frozen_next = frame(game, true);
                assert_eq!(frozen.0, 1.0);
                assert_eq!(frozen_next.0, 1.0);
                assert_ne!(frozen.1, frozen_next.1);
                game.title = Some(Title::new(
                    game.save.root.clone(),
                    &game.options.resources,
                    "",
                ));
                frame(game, false);
            });
        });
    }
}
