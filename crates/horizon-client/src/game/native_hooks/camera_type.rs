use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `key.togglePerspective` (F5) cycles the camera type: `previous` is the type it
    /// leaves, and `next` comes in as `CameraType.cycle` and may be set to another.
    pub(in crate::game) fn native_camera_type_cycle(
        &mut self,
        previous: mod_api::CameraType,
        next: &mut mod_api::CameraType,
    ) {
        self.fire_camera_type_cycle(previous, next);
    }
    /// The window's camera type (`Options.getCameraType`).
    pub(in crate::game) fn camera_type(&self) -> CameraType {
        self.requested_camera_type.get().unwrap_or(self.camera_type)
    }
    /// Queue the window's camera type without cycling, including from shared hooks.
    pub(in crate::game) fn set_camera_type(&self, camera_type: CameraType) {
        self.requested_camera_type.set(Some(camera_type));
    }
}

#[cfg(test)]
mod camera_type_tests {
    use mod_api::{CameraType, CameraTypeCycleEvent, ClientHost, Hooks, ModEntry};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    /// Skips the front view: F5 goes first person, behind, first person.
    struct NoFront(Arc<Mutex<Vec<(CameraType, CameraType)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for NoFront {
        fn camera_type_cycle(
            &self,
            host: &mut dyn ClientHost,
            event: &mut CameraTypeCycleEvent<'_>,
        ) {
            self.0.lock().unwrap().push((event.previous, *event.next));
            host.set_camera_type(CameraType::ThirdPersonFront);
            if *event.next == CameraType::ThirdPersonFront {
                *event.next = CameraType::FirstPerson;
            }
        }
    }

    /// `key.togglePerspective`: the mod hears each cycle with vanilla's next type and its
    /// answer is the new type; setting the type does not cycle, and without a listener F5
    /// cycles as vanilla does.
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_picks_the_camera_type_f5_cycles_to() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.execution_role = super::ExecutionRole::Client;
            let mut seen = Vec::new();
            for _ in 0..3 {
                game.cycle_camera();
                seen.push(game.camera_type());
            }
            use CameraType::*;
            assert_eq!(seen, [ThirdPersonBack, ThirdPersonFront, FirstPerson]);
        });
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "no-front",
            Box::new(NoFront(Arc::clone(&heard))),
        )]
        .into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                use CameraType::*;
                game.options.execution_role = super::ExecutionRole::Client;
                game.cycle_camera();
                assert_eq!(game.camera_type(), ThirdPersonBack);
                game.cycle_camera();
                assert_eq!(game.camera_type(), FirstPerson, "the front view is skipped");
                game.set_camera_type(ThirdPersonFront);
                assert_eq!(game.camera_type(), ThirdPersonFront);
                assert_eq!(game.camera_type, FirstPerson, "the request is pending");
                assert_eq!(heard.lock().unwrap().len(), 2, "setting is not a cycle");
                game.cycle_camera();
                assert_eq!(game.camera_type, FirstPerson, "F5's final choice wins");
            });
        });
        use CameraType::*;
        assert_eq!(
            *heard.lock().unwrap(),
            [
                (FirstPerson, ThirdPersonBack),
                (ThirdPersonBack, ThirdPersonFront),
                (ThirdPersonFront, FirstPerson)
            ],
            "a prior request changes the next F5's previous type without an extra cycle"
        );
    }

    #[derive(Default)]
    struct Calls {
        poses: std::sync::atomic::AtomicUsize,
        cycles: std::sync::atomic::AtomicUsize,
    }
    struct SharedCamera(Arc<Calls>);
    #[mod_api::__subscriptions]
    impl Hooks for SharedCamera {
        fn camera_pose(&self, host: &dyn ClientHost, event: &mut mod_api::CameraPoseEvent<'_>) {
            self.0
                .poses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            host.set_camera_type(CameraType::ThirdPersonFront);
            assert_eq!(host.camera_type(), CameraType::ThirdPersonFront);
            host.set_camera_type(CameraType::ThirdPersonBack);
            assert_eq!(host.camera_type(), CameraType::ThirdPersonBack);
            event.yaw += 15.0;
        }
        fn camera_type_cycle(&self, _: &mut dyn ClientHost, _: &mut CameraTypeCycleEvent<'_>) {
            self.0
                .cycles
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn shared_camera_pose_requests_apply_before_frame_consumers() {
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        use std::sync::atomic::Ordering;

        let calls = Arc::new(Calls::default());
        let mods: mod_api::Mods = vec![ModEntry::new(
            "shared-camera",
            Box::new(SharedCamera(Arc::clone(&calls))),
        )]
        .into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
                game.title = None;
                game.spectated_post_effect = Some("minecraft:creeper");
                let vanilla = game.camera_view(1.0).unwrap();
                assert_eq!(vanilla.0.z, 0.0);
                let posed = game.native_camera_pose_now(1.0, vanilla);
                assert_eq!(game.camera_type, CameraType::ThirdPersonBack);
                assert_eq!(game.camera_type(), CameraType::ThirdPersonBack);
                assert_eq!(game.spectated_post_effect, None);
                assert!((posed.0.z + 4.0).abs() < 1e-6, "refresh unchanged position");
                assert_eq!(posed.1, 15.0, "preserve the explicit yaw override");
                assert_eq!(posed.2, vanilla.2);
            });
        });
        assert_eq!(calls.poses.load(Ordering::Relaxed), 1);
        assert_eq!(calls.cycles.load(Ordering::Relaxed), 0);
    }
}
