//! The explicit run deadline (`Options::time_limit`, game/runtime/launch_options.rs) ends a
//! live run.
#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[test]
    #[ignore = "daily"]
    fn explicit_deadline_exits_the_live_idle_path_without_frame_limit() {
        use crate::game::event_loop::EventLoopCtl;
        use std::cell::Cell;
        use winit::event_loop::ControlFlow;

        struct TestLoop(Cell<bool>);
        impl EventLoopCtl for TestLoop {
            fn exit(&self) {
                self.0.set(true);
            }
            fn set_control_flow(&self, _: ControlFlow) {}
        }

        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.frames = None;
            game.options.world_frames = None;
            game.options.timeout = Some(Duration::from_secs(1));
            game.started = std::time::Instant::now() - Duration::from_secs(2);
            let event_loop = TestLoop(Cell::new(false));
            game.idle(&event_loop);
            assert!(event_loop.0.get(), "the live idle path must request exit");
            assert_eq!(
                game.error.as_ref().map(ToString::to_string).as_deref(),
                Some("run timed out after 1 s")
            );
        });
    }
}
