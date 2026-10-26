//! Extras that are official mods now (`horizon_client::moved_extras`): on
//! the first start after the update, a player who had one on gets the mod
//! installed from Horizon's registry, turned on, with their settings, and
//! one line in the chat saying so. The download runs on its own thread; if
//! it fails (offline), the game tries again at the next start.

#[cfg(not(feature = "mods"))]
impl super::Game<'_> {
    pub(super) fn start_moved_extras(&mut self) {}
    pub(super) fn poll_moved_extras(&mut self) {}
}

#[cfg(feature = "mods")]
mod with_mods {
    use super::super::Game;
    use horizon_client::{
        moved_extras::{self, Moved},
        text::{self, Text},
    };
    use std::sync::mpsc::{Receiver, channel};

    type Done = Vec<(&'static Moved, Result<(), String>)>;

    #[derive(Default)]
    pub(in crate::game) struct MovedState {
        /// The installs under way.
        running: Option<Receiver<Done>>,
        /// Lines for the player, shown once a world is open.
        notices: Vec<Text>,
    }

    impl Game<'_> {
        /// At start: remembers the moved extras that were off, and installs
        /// the ones that were on, in the background.
        pub(in crate::game) fn start_moved_extras(&mut self) {
            let off = moved_extras::nothing_to_do(&self.settings.extras);
            let pending = moved_extras::pending(&self.settings.extras);
            if off.is_empty() && pending.is_empty() {
                return;
            }
            if !off.is_empty() {
                let e = &mut self.settings.extras;
                e.moved.extend(off.iter().map(|m| m.id.to_owned()));
                self.settings.save(&Self::options_path());
            }
            if pending.is_empty() {
                return;
            }
            let extras = self.settings.extras.clone();
            let dir = Self::options_path()
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_default();
            let (tx, rx) = channel();
            self.mods.moved.running = Some(rx);
            let spawned = std::thread::Builder::new()
                .name("Moved extras".into())
                .spawn(move || {
                    let done = match moved_extras::fetch_index() {
                        Ok(index) => pending
                            .into_iter()
                            .map(|m| (m, moved_extras::install(&index, m, &extras, &dir)))
                            .collect(),
                        Err(e) => pending.into_iter().map(|m| (m, Err(e.clone()))).collect(),
                    };
                    let _ = tx.send(done);
                });
            if spawned.is_err() {
                self.mods.moved.running = None;
            }
        }
        /// Once a frame: the installs that finished turn their mods on;
        /// their lines wait for a world.
        pub(in crate::game) fn poll_moved_extras(&mut self) {
            if let Some(rx) = &self.mods.moved.running
                && let Ok(done) = rx.try_recv()
            {
                self.mods.moved.running = None;
                let mut installed = false;
                for (m, result) in done {
                    match result {
                        Ok(()) => {
                            self.settings.extras.moved.push(m.id.to_owned());
                            self.settings.mods.enabled.insert(m.id.to_owned());
                            // The install wrote the key to horizon-mods.txt;
                            // the settings save below must keep it.
                            let keys = self.settings.mods.keys.entry(m.id.to_owned()).or_default();
                            for (name, key) in m.keys(&self.settings.extras) {
                                keys.entry(name.to_owned()).or_insert(key);
                            }
                            self.settings.mods.keys.retain(|_, k| !k.is_empty());
                            self.mods.moved.notices.push(Text::literal(m.notice()));
                            installed = true;
                        }
                        Err(e) => {
                            log::warn!("Horizon: {} not installed, next start retries: {e}", m.id);
                            self.mods.moved.notices.push(
                                Text::literal(format!(
                                    "{} moved to the mod browser; it couldn't be installed for you yet, Horizon tries again at the next start",
                                    m.name
                                ))
                                .with_color(text::GRAY),
                            );
                        }
                    }
                }
                if installed {
                    self.settings.save(&Self::options_path());
                    self.start_mods();
                }
            }
            if !self.mods.moved.notices.is_empty() && self.player.is_some() && self.title.is_none()
            {
                for line in std::mem::take(&mut self.mods.moved.notices) {
                    self.system_message(line);
                }
            }
        }
    }
}
#[cfg(feature = "mods")]
pub(super) use with_mods::MovedState;
