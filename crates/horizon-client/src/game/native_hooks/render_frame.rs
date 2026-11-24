use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_render_frame(&self, gui_only: &mut bool) {
        self.fire_render_frame(gui_only);
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{
        Game,
        native_entity_refs::tests::{run_fresh_cases, with_client_game},
        native_hooks::particle_count_tests::frame_scene,
    };
    use horizon_client::{gfx::ScreenshotRequest, graphics::HdrMode};
    use mod_api::{ClientHost, Flow, Hooks, ModEntry, RenderFrameEvent, ScreenRenderPostEvent};
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };

    struct FullMap(Arc<AtomicBool>);
    const MOD_ID: &str = "fullmap";

    #[mod_api::__subscriptions]
    impl Hooks for FullMap {
        fn render_frame(&self, host: &dyn ClientHost, _: &mut RenderFrameEvent) -> Flow {
            if host.mod_screen() == Some(MOD_ID) && self.0.load(Ordering::Relaxed) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }

        fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
            if host.mod_screen() != Some(MOD_ID) || !self.0.load(Ordering::Relaxed) {
                return;
            }
            let [w, h] = event.draw.size();
            let [x, y] = [w * 0.5 - 8.0, h * 0.5 - 8.0];
            // Leave a small hole in the full-map background to expose its backdrop.
            for rect in [
                [0.0, 0.0, w, y],
                [0.0, y + 16.0, w, y],
                [0.0, y, x, 16.0],
                [x + 16.0, y, x, 16.0],
            ] {
                event.draw.fill(rect, 0xffff_00ff);
            }
        }
    }

    fn draw(game: &mut Game<'_>) {
        game.extras_frame(Duration::from_millis(16));
        assert!(
            game.render_with(|renderer, frame| {
                renderer.set_keep_timed_frames(true);
                renderer.render(frame)
            })
            .unwrap()
        );
        game.finish_render_frame(Duration::from_millis(16));
        let renderer = game.renderer.as_mut().unwrap();
        renderer.wait_in_flight();
        renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }

    /// Read pixels from the first redraw, and a GPU timing sample under the same conditions.
    /// Timestamp results go through a second submit and a readback on later redraws.
    fn sample(game: &mut Game<'_>) -> ([u8; 3], [u8; 3], Vec<String>) {
        let began = Instant::now();
        game.renderer.as_mut().unwrap().screenshot = Some(ScreenshotRequest {
            path: "unused.png".into(),
            size: None,
            scaled: false,
            background: false,
        });
        draw(game);
        let mut result = game
            .renderer
            .as_mut()
            .unwrap()
            .screenshot_result
            .take()
            .unwrap()
            .unwrap();
        let image = result.image_mut();
        let pixel = |x, y| {
            let offset = (y * image.width + x) as usize * 4;
            image.rgba[offset..offset + 3].try_into().unwrap()
        };
        let center = pixel(image.width / 2, image.height / 2);
        let map = pixel(image.width / 4, image.height / 4);
        for _ in 0..3 {
            draw(game);
        }
        let timing = game
            .renderer
            .as_mut()
            .unwrap()
            .take_timed_frames()
            .own
            .into_iter()
            .find(|frame| frame.at >= began)
            .expect("the GPU must return frame timestamps");
        (
            center,
            map,
            timing.passes.into_iter().map(|(name, _)| name).collect(),
        )
    }

    #[test]
    #[ignore = "daily"]
    fn a_mod_map_presents_without_the_world_pass() {
        let name =
            "game::native_hooks::render_frame::tests::a_mod_map_presents_without_the_world_pass";
        if run_fresh_cases(name, "HORIZON_FRAME_GUI_ONLY_CHILD", &["pixels"]) {
            return;
        }
        with_client_game(|game| {
            let hdr = std::env::var_os("HORIZON_HDR_OUTPUT").is_some();
            if hdr {
                game.settings.horizon.hdr = HdrMode::On;
            }
            let _event_loop = frame_scene(game);
            game.ui.hide_gui = true; // F1 leaves screens visible, without the crosshair.
            game.gpu_passes = true;
            game.yaw = 0.0;
            game.pitch = 0.0;
            // Apply the requested output before sampling the stock sky in the world pass.
            game.apply_ui_settings();
            draw(game);
            let renderer = game.renderer.as_ref().unwrap();
            let output = renderer.output();
            assert_eq!(output.hdr_on(), hdr, "{output:?}");
            assert!(game.mod_screen_open(MOD_ID));

            let (stock, _, passes) = sample(game);
            assert!(stock[2] > stock[0] && stock[0] > 30, "{stock:?}");
            assert!(passes.iter().any(|name| name == "World"), "{passes:?}");
            let request = Arc::new(AtomicBool::new(false));
            let mods = vec![ModEntry::new(MOD_ID, Box::new(FullMap(request.clone())))];
            mod_platform::testing::with_mods(mods.into(), || {
                let (declined, _, passes) = sample(game);
                assert_eq!(declined, stock);
                assert!(passes.iter().any(|name| name == "World"), "{passes:?}");

                request.store(true, Ordering::Relaxed);
                let (center, map, passes) = sample(game);
                assert_eq!(map, [255, 0, 255], "the mod screen's GPU-drawn background");
                assert_eq!(center, [0; 3], "the unpainted hole must not show the world");
                assert!(
                    passes.iter().any(|name| name == "GUI background"),
                    "{passes:?}"
                );
                assert!(!passes.iter().any(|name| name == "World"), "{passes:?}");

                request.store(false, Ordering::Relaxed);
                let (restored, _, passes) = sample(game);
                assert_eq!(
                    restored, stock,
                    "the first world redraw restores the visible sky"
                );
                assert!(passes.iter().any(|name| name == "World"), "{passes:?}");
                println!(
                    "HDR={hdr}; stock={stock:?}; declined={declined:?}; \
                     map={map:?} over {center:?} (no World pass); restored={restored:?}"
                );
            });
        });
    }
}
