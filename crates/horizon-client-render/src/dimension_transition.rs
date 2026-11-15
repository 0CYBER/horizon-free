//! Load-time dimension-key resolution and selection of client loading presentations.
use mod_api::{DimensionKey, DimensionTransition, DimensionTransitionScreen, Identifier};
use std::{collections::HashMap, sync::OnceLock};

/// Registrations resolved once at load; lookups never build dimension names.
#[derive(Debug, Default)]
pub struct TransitionScreens {
    conditional: HashMap<(DimensionKey, DimensionKey), usize>,
    incoming: HashMap<DimensionKey, usize>,
    outgoing: HashMap<DimensionKey, usize>,
    screens: Vec<String>,
}

static SCREENS: OnceLock<TransitionScreens> = OnceLock::new();

impl TransitionScreens {
    /// Validate registrations, resolving dimension names to process-owned keys.
    pub fn new(registrations: Vec<DimensionTransitionScreen>) -> Self {
        let mut result = Self::default();
        for registration in registrations {
            if !valid_id(&registration.screen) || registration.screen.starts_with("minecraft:") {
                log::warn!(
                    "Invalid dimension transition screen id: {}",
                    registration.screen
                );
                continue;
            }
            let slot = result.screens.len();
            let accepted = match registration.transition {
                DimensionTransition::Incoming(to) => {
                    resolve(&to).is_some_and(|key| insert(&mut result.incoming, key, slot))
                }
                DimensionTransition::Outgoing(from) => {
                    resolve(&from).is_some_and(|key| insert(&mut result.outgoing, key, slot))
                }
                DimensionTransition::Conditional { to, from } => resolve(&to)
                    .zip(resolve(&from))
                    .is_some_and(|pair| insert(&mut result.conditional, pair, slot)),
            };
            if accepted {
                result.screens.push(registration.screen);
            } else {
                log::warn!(
                    "Invalid or duplicate dimension transition registration: {}",
                    registration.screen
                );
            }
        }
        result
    }

    /// Conditional, then incoming, then outgoing, as NeoForge's event javadoc specifies.
    pub fn select(&self, to: DimensionKey, from: Option<DimensionKey>) -> Option<usize> {
        from.and_then(|from| self.conditional.get(&(to, from)))
            .or_else(|| self.incoming.get(&to))
            .or_else(|| from.and_then(|from| self.outgoing.get(&from)))
            .copied()
    }

    /// The selected screen's id; the slot stays valid for this registration table's lifetime.
    pub fn screen(&self, slot: usize) -> Option<&str> {
        self.screens.get(slot).map(String::as_str)
    }
}

fn insert<K: Eq + std::hash::Hash>(table: &mut HashMap<K, usize>, key: K, slot: usize) -> bool {
    if let std::collections::hash_map::Entry::Vacant(entry) = table.entry(key) {
        entry.insert(slot);
        true
    } else {
        false
    }
}

fn valid_id(id: &str) -> bool {
    Identifier::parse_namespaced(id).is_some()
}

fn resolve(id: &str) -> Option<DimensionKey> {
    valid_id(id).then(|| mod_api::__private::dimension_key(id))
}

/// Publish the client registrations once at startup.
pub fn register(registrations: Vec<DimensionTransitionScreen>) {
    if SCREENS.set(TransitionScreens::new(registrations)).is_err() {
        log::warn!("Dimension transition screens already registered; keeping the first table");
    }
}

/// Choose once at dimension entry. With no registrations there is no lookup or allocation.
pub fn select(to: DimensionKey, from: Option<DimensionKey>) -> Option<usize> {
    SCREENS.get().and_then(|screens| screens.select(to, from))
}

/// Read the retained presentation when drawing the loading screen.
pub fn screen(slot: usize) -> Option<&'static str> {
    SCREENS.get().and_then(|screens| screens.screen(slot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level_loading::{LevelLoadTracker, PlayerCheck, Reason, ServerLoadView, Stage};

    fn key(id: &str) -> DimensionKey {
        mod_api::__private::dimension_key(id)
    }
    fn entry(screen: &str, transition: DimensionTransition) -> DimensionTransitionScreen {
        DimensionTransitionScreen {
            screen: screen.into(),
            transition,
        }
    }
    fn chosen<'a>(screens: &'a TransitionScreens, to: &str, from: Option<&str>) -> Option<&'a str> {
        screens
            .select(key(to), from.map(key))
            .and_then(|slot| screens.screen(slot))
    }

    #[test]
    fn conditional_wins_over_incoming_then_outgoing_and_first_registration_wins() {
        let screens = TransitionScreens::new(vec![
            entry("test:leave", DimensionTransition::Outgoing("test:a".into())),
            entry(
                "test:arrive",
                DimensionTransition::Incoming("test:b".into()),
            ),
            entry(
                "test:pair",
                DimensionTransition::Conditional {
                    to: "test:b".into(),
                    from: "test:a".into(),
                },
            ),
            entry(
                "test:duplicate",
                DimensionTransition::Conditional {
                    to: "test:b".into(),
                    from: "test:a".into(),
                },
            ),
            entry(
                "test:duplicate",
                DimensionTransition::Incoming("test:b".into()),
            ),
            entry(
                "test:duplicate",
                DimensionTransition::Outgoing("test:a".into()),
            ),
        ]);
        assert_eq!(
            chosen(&screens, "test:b", Some("test:a")),
            Some("test:pair")
        );
        assert_eq!(
            chosen(&screens, "test:b", Some("test:c")),
            Some("test:arrive")
        );
        assert_eq!(
            chosen(&screens, "test:c", Some("test:a")),
            Some("test:leave")
        );
        assert_eq!(chosen(&screens, "test:b", None), Some("test:arrive"));
        assert_eq!(chosen(&screens, "test:c", Some("test:b")), None);
        assert_eq!(chosen(&screens, "test:c", None), None);
    }

    #[test]
    fn invalid_ids_do_not_reserve_a_transition_or_replace_vanilla() {
        let screens = TransitionScreens::new(vec![
            entry(
                "minecraft:reserved",
                DimensionTransition::Incoming("test:b".into()),
            ),
            entry(
                "missing_namespace",
                DimensionTransition::Incoming("test:b".into()),
            ),
            entry(
                "test:invalid",
                DimensionTransition::Incoming("INVALID:b".into()),
            ),
            entry(
                "test:invalid",
                DimensionTransition::Outgoing("missing_namespace".into()),
            ),
            entry(
                "test:invalid",
                DimensionTransition::Conditional {
                    to: "test:b".into(),
                    from: "INVALID:a".into(),
                },
            ),
            entry("test:valid", DimensionTransition::Incoming("test:b".into())),
        ]);
        assert_eq!(
            chosen(&screens, "test:b", Some("test:a")),
            Some("test:valid")
        );
        assert_eq!(chosen(&screens, "test:c", Some("test:a")), None);
        assert_eq!(
            chosen(&TransitionScreens::default(), "test:b", Some("test:a")),
            None
        );
    }

    #[test]
    fn selected_screen_follows_existing_progress_readiness_and_cleanup_then_falls_back() {
        let screens = TransitionScreens::new(vec![entry(
            "test:arrival",
            DimensionTransition::Conditional {
                to: "test:b".into(),
                from: "test:a".into(),
            },
        )]);
        let player = PlayerCheck {
            outside_build_height: false,
            spectator: false,
            alive: true,
        };
        let mut tracker = LevelLoadTracker::new(500);
        tracker.start_client_load(0, Reason::Other);
        tracker.set_transition_screen(screens.select(key("test:b"), Some(key("test:a"))));
        assert_eq!(
            tracker
                .transition_screen()
                .and_then(|slot| screens.screen(slot)),
            Some("test:arrival")
        );
        let progress = ServerLoadView::default();
        progress.start(Stage::LoadInitialChunks, 0);
        progress.finish(Stage::LoadInitialChunks);
        progress.start(Stage::LoadPlayerChunks, 49);
        progress.update(Stage::LoadPlayerChunks, 20, 49);
        assert_eq!(progress.server_progress(), 30.0 / 59.0);
        tracker.tick_client_load(100, player);
        assert!(!tracker.is_level_ready(100));
        tracker.loading_packets_received();
        tracker.tick_client_load(150, player);
        assert!(!tracker.is_level_ready(150));
        tracker.player_section_ready();
        tracker.tick_client_load(200, player);
        assert!(!tracker.is_level_ready(699));
        assert!(tracker.is_level_ready(700));
        tracker.finish();
        assert_eq!(tracker.transition_screen(), None);
        assert!(!tracker.is_active());
        tracker.start_client_load(1000, Reason::Other);
        tracker.set_transition_screen(screens.select(key("test:c"), Some(key("test:b"))));
        assert_eq!(tracker.transition_screen(), None);
        tracker.loading_packets_received();
        tracker.player_section_ready();
        tracker.tick_client_load(1100, player);
        assert!(!tracker.is_level_ready(1599));
        assert!(tracker.is_level_ready(1600));
    }

    #[test]
    fn replacement_abort_and_timeout_keep_the_tracker_in_charge() {
        let mut tracker = LevelLoadTracker::new(0);
        tracker.start_client_load(0, Reason::NetherPortal);
        tracker.set_transition_screen(Some(0));
        tracker.start_client_load(100, Reason::EndPortal);
        assert_eq!(tracker.transition_screen(), None);
        tracker.set_transition_screen(Some(1));
        tracker.finish();
        assert_eq!(tracker.transition_screen(), None);
        tracker.set_transition_screen(Some(2));
        assert_eq!(tracker.transition_screen(), None);
        tracker.start_client_load(200, Reason::Other);
        tracker.set_transition_screen(Some(3));
        tracker.loading_packets_received();
        tracker.tick_client_load(
            30201,
            PlayerCheck {
                outside_build_height: false,
                spectator: false,
                alive: true,
            },
        );
        assert!(tracker.is_level_ready(30201));
        tracker.finish();
        assert_eq!(tracker.transition_screen(), None);
    }
}
