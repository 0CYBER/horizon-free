//! Client loading-screen registrations and their presentation data.

/// A screen registered by `Hooks::register_dimension_transition_screen`.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: mods construct registrations and the game reads both fields
#[derive(horizon_mod_api_macros::Wire)]
pub struct DimensionTransitionScreen {
    /// Namespaced screen id, drawn by `Hooks::dimension_transition_screen_draw`.
    pub screen: String,
    /// Which travel selects this screen.
    pub transition: DimensionTransition,
}

/// NeoForge's incoming, outgoing and conditional registration variants.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the three registration variants
#[derive(horizon_mod_api_macros::Wire)]
pub enum DimensionTransition {
    /// Travel to this dimension (`registerIncomingEffect`).
    Incoming(String),
    /// Travel from this dimension (`registerOutgoingEffect`).
    Outgoing(String),
    /// Travel to `to` from `from` (`registerConditionalEffect`), highest priority.
    Conditional {
        /// Destination dimension id.
        to: String,
        /// Source dimension id.
        from: String,
    },
}

/// Why the client is displaying its level-loading screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: LevelLoadingScreen.Reason's three variants
#[derive(horizon_mod_api_macros::Wire)]
pub enum DimensionTransitionReason {
    /// Travel involving the Nether, except a death respawn.
    NetherPortal,
    /// Travel involving the End, except a death respawn or Nether travel.
    EndPortal,
    /// Login, death respawn, or other travel.
    Other,
}

#[cfg(test)]
mod dimension_transition_tests {
    use crate::*;

    struct Client;
    impl Host for Client {}
    crate::impl_host_areas!(Client);
    impl ClientHost for Client {}
    crate::impl_client_host_areas!(Client);

    #[derive(Default)]
    struct Canvas(Vec<([f32; 4], u32)>);
    impl DrawContext for Canvas {
        fn size(&self) -> [f32; 2] {
            [320.0, 240.0]
        }
        fn fill(&mut self, rect: [f32; 4], color: u32) {
            self.0.push((rect, color));
        }
    }

    struct Arrival;
    #[crate::__subscriptions]
    impl Hooks for Arrival {
        fn register_dimension_transition_screen(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterDimensionTransitionScreenEvent<'_>,
        ) {
            event.screens.push(DimensionTransitionScreen {
                screen: "test:arrival".into(),
                transition: DimensionTransition::Conditional {
                    to: "test:b".into(),
                    from: "test:a".into(),
                },
            });
        }
        fn dimension_transition_screen_draw(
            &self,
            _: &dyn ClientHost,
            event: &mut DimensionTransitionScreenDrawEvent<'_>,
        ) {
            if event.screen != "test:arrival" {
                return;
            }
            let [w, h] = event.draw.size();
            event.draw.fill([0.0, 0.0, w, h], 0xFF102030);
            if event.reason == DimensionTransitionReason::NetherPortal {
                event.draw.fill(
                    [0.0, 230.0, w * event.progress.unwrap_or(0.0), 10.0],
                    0xFF00FF00,
                );
            }
        }
    }

    #[test]
    fn dimension_transition_registration_and_draw_carry_selection_reason_and_progress() {
        testing::with_dispatch(
            vec![ModEntry::new("test", Box::new(Arrival))].into(),
            || {
                let mut screens = Vec::new();
                dispatch_register_dimension_transition_screen(
                    &RegistrationContext::new(),
                    &mut screens,
                );
                assert_eq!(
                    screens,
                    [DimensionTransitionScreen {
                        screen: "test:arrival".into(),
                        transition: DimensionTransition::Conditional {
                            to: "test:b".into(),
                            from: "test:a".into()
                        },
                    }]
                );
                let mut canvas = Canvas::default();
                dispatch_dimension_transition_screen_draw(
                    &Client,
                    "test:arrival",
                    DimensionTransitionReason::NetherPortal,
                    Some(0.25),
                    &mut canvas,
                );
                assert_eq!(
                    canvas.0,
                    [
                        ([0.0, 0.0, 320.0, 240.0], 0xFF102030),
                        ([0.0, 230.0, 80.0, 10.0], 0xFF00FF00)
                    ]
                );
                let mut canvas = Canvas::default();
                dispatch_dimension_transition_screen_draw(
                    &Client,
                    "test:other",
                    DimensionTransitionReason::Other,
                    None,
                    &mut canvas,
                );
                assert!(canvas.0.is_empty());
            },
        );
    }

    #[test]
    fn dimension_transition_without_listeners_collects_and_draws_nothing() {
        testing::with_dispatch(Vec::new().into(), || {
            let mut screens = Vec::new();
            dispatch_register_dimension_transition_screen(
                &RegistrationContext::new(),
                &mut screens,
            );
            assert!(screens.is_empty());
            let mut canvas = Canvas::default();
            dispatch_dimension_transition_screen_draw(
                &Client,
                "test:arrival",
                DimensionTransitionReason::Other,
                None,
                &mut canvas,
            );
            assert!(canvas.0.is_empty());
        });
    }
}
