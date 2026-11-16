use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Once for each frame presented, after `render_frame_post` (`FlipFrameEvent`).
    pub(in crate::game) fn native_flip_frame(&mut self) {
        self.fire_flip_frame();
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
    use super::super::particle_count_tests::frame_scene;
    use mod_api::{
        ClientHost, FlipFrameEvent, Hooks, ModEntry, RenderFramePostEvent, RenderFramePreEvent,
    };
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct Moments(Arc<Mutex<Vec<&'static str>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Moments {
        fn render_frame_pre(&self, _: &mut dyn ClientHost, _: &mut RenderFramePreEvent) {
            self.0.lock().unwrap().push("pre");
        }
        fn render_frame_post(&self, _: &mut dyn ClientHost, _: &mut RenderFramePostEvent) {
            self.0.lock().unwrap().push("post");
        }
        fn flip_frame(&self, _: &mut dyn ClientHost, _: &mut FlipFrameEvent) {
            self.0.lock().unwrap().push("flip");
        }
    }

    #[test]
    #[ignore = "daily"]
    fn flip_frame_follows_render_frame_post_on_presented_frames_only() {
        let name = "game::native_hooks::flip_frame::tests::flip_frame_follows_render_frame_post_on_presented_frames_only";
        if run_fresh_cases(name, "HORIZON_FLIP_FRAME_CHILD", &["frames"]) {
            return;
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("moments", Box::new(Moments(heard.clone())))];
        mod_platform::testing::with_mods(mods.into(), || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                let elapsed = Duration::from_millis(16);
                let frame = |game: &mut super::Game<'_>, shown, moments: &[&str]| {
                    game.extras_frame(elapsed);
                    assert_eq!(game.render_and_flip(elapsed).unwrap(), shown);
                    assert_eq!(*heard.lock().unwrap(), moments);
                    heard.lock().unwrap().clear();
                };

                frame(game, true, &["pre", "post", "flip"]);

                game.title = Some(game.new_title(game.save.root.clone()));
                game.render_with(|_, frame| {
                    assert!(frame.title);
                    Ok(false)
                })
                .unwrap();
                frame(game, true, &["pre", "post", "flip"]);

                // A minimized window: the frame is prepared, never drawn.
                game.renderer.as_mut().unwrap().suspended = true;
                frame(game, false, &["pre", "post"]);
                game.title = None;
                frame(game, false, &["pre", "post"]);
            });
        });
    }
}
