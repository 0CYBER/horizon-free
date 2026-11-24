use super::*;

impl Game<'_> {
    /// Offer only the saving-world GUI; the save state machine does not read the answer.
    pub(in crate::game) fn native_closing_save_screen_draw(
        &self,
        operation: mod_api::ClosingSaveOperation,
        stage: mod_api::ClosingSaveStage,
        progress: Option<f32>,
        draw: &mut HudBatch,
        replacement: &mut Option<()>,
    ) {
        self.fire_closing_save_screen_draw(operation, stage, progress, draw, replacement);
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
    use horizon_client::gfx::ScreenshotRequest;
    use horizon_client::save::{Level, WorldPreset};
    use horizon_client::title::WorldOptions;
    use mod_api::{
        ClientHost, ClosingSaveOperation, ClosingSaveScreenDrawEvent, ClosingSaveStage, Hooks,
        ModEntry, Mods,
    };
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    type Observation = (ClosingSaveOperation, ClosingSaveStage, Option<f32>, [f32; 2]);

    #[derive(Clone, Default)]
    struct Painter(Arc<PaintState>);

    #[derive(Default)]
    struct PaintState {
        replace: AtomicBool,
        draws: Mutex<Vec<Observation>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Painter {
        #[listener(priority = 10)]
        fn closing_save_screen_draw(
            &self,
            _: &dyn ClientHost,
            event: &mut ClosingSaveScreenDrawEvent<'_>,
        ) -> Option<()> {
            let [width, height] = event.draw.size();
            self.0.draws.lock().unwrap().push((
                event.operation,
                event.stage,
                event.progress,
                [width, height],
            ));
            if self.0.replace.load(Relaxed) {
                event.draw.fill([0.0, 0.0, width, height], 0xffff00ff);
                Some(())
            } else {
                None
            }
        }
    }

    #[mod_api::__subscriptions]
    impl Hooks for Last {
        fn closing_save_screen_draw(
            &self,
            _: &dyn ClientHost,
            _: &mut ClosingSaveScreenDrawEvent<'_>,
        ) -> Option<()> {
            self.0.fetch_add(1, Relaxed);
            None
        }
    }

    #[derive(Clone, Default)]
    struct Last(Arc<AtomicUsize>);

    fn capture(game: &mut super::Game<'_>) -> (Vec<u8>, [u32; 2]) {
        game.renderer.as_mut().unwrap().screenshot = Some(ScreenshotRequest {
            path: "unused.png".into(),
            size: None,
            scaled: false,
            background: false,
        });
        game.render_closing_save().unwrap();
        let mut pending = game
            .renderer
            .as_mut()
            .unwrap()
            .screenshot_result
            .take()
            .expect("saving frame did not reach the renderer")
            .unwrap();
        let image = pending.image_mut();
        (std::mem::take(&mut image.rgba), [image.width, image.height])
    }

    #[test]
    #[ignore = "daily"]
    fn leaving_and_quitting_can_replace_the_presented_save_but_not_the_write() {
        let name = "game::native_hooks::closing_save_screen_draw::tests::leaving_and_quitting_can_replace_the_presented_save_but_not_the_write";
        const CASE: &str = "HORIZON_CLOSING_SAVE_SCREEN_CHILD";
        if run_fresh_cases(name, CASE, &["leave", "quit", "control"]) {
            return;
        }
        let case = std::env::var(CASE).unwrap();
        let quit = case == "quit";
        let operation = if quit {
            ClosingSaveOperation::QuitClient
        } else {
            ClosingSaveOperation::LeaveWorld
        };
        with_client_game(|game| {
            let mut builder = winit::event_loop::EventLoop::builder();
            #[cfg(target_os = "linux")]
            {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_x11().with_any_thread(true);
            }
            #[cfg(target_os = "windows")]
            {
                use winit::platform::windows::EventLoopBuilderExtWindows;
                builder.with_any_thread(true);
            }
            let event_loop = builder.build().unwrap();
            #[allow(deprecated)]
            let window = Arc::new(
                event_loop
                    .create_window(
                        winit::window::Window::default_attributes()
                            .with_visible(false)
                            .with_inner_size(winit::dpi::PhysicalSize::new(640, 480)),
                    )
                    .unwrap(),
            );
            game.options.offscreen = true;
            game.settings.gui_scale = 2;
            game.settings.render_distance = 2;
            game.settings.simulation_distance = 2;
            game.open_integrated(
                "closing",
                Some("Closing save"),
                Some(1),
                true,
                Some(&WorldOptions {
                    world_type: WorldPreset::Flat,
                    ..Default::default()
                }),
            )
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            while !game.remote.as_ref().is_some_and(|remote| remote.joined) {
                game.poll_remote().unwrap();
                assert!(Instant::now() < deadline, "could not join the local world");
                std::thread::sleep(Duration::from_millis(5));
            }
            let renderer = game.new_renderer(window).unwrap();
            // Keep the live server in one task, with a new spawn that cannot be persisted
            // until we let it stop. Both render paths draw the same real pending save.
            let (entered, ready) = mpsc::channel();
            let (release, wait) = mpsc::channel::<()>();
            let spawn = [53, 100, -41];
            assert!(game.integrated.as_ref().unwrap().post(move |server| {
                server.set_world_spawn(spawn, [0.0; 2]);
                entered.send(()).unwrap();
                let _ = wait.recv_timeout(Duration::from_secs(30));
            }));
            ready.recv_timeout(Duration::from_secs(10)).unwrap();
            let saved = Path::new("saves/closing");
            assert_ne!(
                Level::read(saved).and_then(|level| level.world_spawn),
                Some(spawn)
            );
            if quit {
                game.begin_closing_save(true).unwrap();
            } else {
                game.close_world().unwrap();
            }
            assert!(!game.poll_closing_save().unwrap());
            assert!(game.pending_save.is_some());

            let painter = Painter::default();
            let last = Last::default();
            let mods: Mods = vec![
                ModEntry::new("closing_painter", Box::new(painter.clone())),
                ModEntry::new("closing_last", Box::new(last.clone())),
            ]
            .into();
            if case != "control" {
                mod_platform::testing::with_mods(mods.clone(), || {
                    game.render_closing_save().unwrap();
                    assert!(painter.0.draws.lock().unwrap().is_empty(), "no renderer");
                });
            }
            game.renderer = Some(renderer);
            let (default, _) = capture(game);
            let magenta = |pixel: &[u8; 4]| pixel[..3] == [255, 0, 255];
            let pixels = default.as_chunks::<4>().0;
            assert!(pixels.iter().any(|pixel| pixel != &pixels[0]));
            if case != "control" {
                mod_platform::testing::with_mods(mods, || {
                    assert_eq!(capture(game).0, default, "declining changed the frame");
                    painter.0.replace.store(true, Relaxed);
                    let (replaced, size) = capture(game);
                    assert_eq!(size, [640, 480]);
                    assert_eq!(replaced.len(), 640 * 480 * 4);
                    assert!(
                        replaced.as_chunks::<4>().0.iter().all(magenta),
                        "default contents survived or the replacement did not reach the target"
                    );
                });
                assert_eq!(
                    last.0.load(Relaxed),
                    1,
                    "a later listener ran after a claim"
                );
                assert_eq!(
                    *painter.0.draws.lock().unwrap(),
                    [(operation, ClosingSaveStage::Server, None, [320.0, 240.0]); 2]
                );
            }
            assert!(!game.poll_closing_save().unwrap());
            assert!(game.pending_save.is_some(), "drawing ended the save");
            assert_ne!(
                Level::read(saved).and_then(|level| level.world_spawn),
                Some(spawn)
            );

            release.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            while game.pending_save.is_some() {
                let exit = game.poll_closing_save().unwrap();
                assert_eq!(exit, quit && game.pending_save.is_none());
                assert!(Instant::now() < deadline, "the world did not finish saving");
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(game.integrated.is_none());
            assert_eq!(Level::read(saved).unwrap().world_spawn, Some(spawn));
        });
    }
}
