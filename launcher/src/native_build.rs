use super::*;
use horizon_launcher::native_mods::prepare_first::{self, Next, Snapshot, Waiting};

impl Launcher {
    /// Records why a build failed. A mod's own compile failure reads as plain words with a way
    /// on; the compiler's text stays in the build log. `enabled` are the mods that were built.
    pub(super) fn native_failed(&mut self, error: String, enabled: &[String]) {
        let (text, failed) = failure_message(error, enabled);
        self.native_error = Some(text);
        if !failed.is_empty() {
            self.native_failed_mods = failed;
        }
    }

    /// Builds the selected instance's native mods, once the player accepted the warning. The
    /// build stays on the Horizon the instance plays (`native_mods::base_for`), unless the
    /// warning was Update's: then it moves to that newer Horizon.
    pub(super) fn build_native(&mut self) {
        self.build_native_on(None, false)
    }

    /// [`Self::build_native`] on `base` when given (an update's newer release or Nightly) instead
    /// of the Horizon the instance plays; with `back_up`, the player chose to back up the worlds
    /// saved with mods the build switches off, then build without those mods.
    pub(super) fn build_native_on(&mut self, base: Option<String>, back_up: bool) {
        if self.native_step.is_some() {
            return;
        }
        // Every way to a build asks first while native mods aren't allowed (permission taken
        // back in Storage, say, with a card still open): the question, never a refusal.
        // The card stays under the question: Cancel goes back to it.
        if self.settings.native_mods_allowed_at.is_none() {
            self.request_native_build_consent();
            self.native_consent_build = Some((base, back_up));
            return;
        }
        self.pending_consent = None;
        self.native_error = None;
        self.native_failed_mods.clear();
        self.native_worlds = None;
        self.mods.unbuilt = None;
        // This attempt's own log, starting empty: an open panel shows it rather than the last
        // attempt's (`native_log_page`).
        self.native_log_reset();
        let instance = instances::selected();
        self.native_for = Some(instance.id.clone());
        let plays = self.version_of(&instance).cloned();
        let base = base.or_else(|| native_mods::base_for(plays.as_deref(), &self.installed, false));
        let Some(base) = base else {
            let error = tr("Install Horizon first.").to_string();
            self.native_log_preflight(&error);
            self.native_error = Some(error);
            return;
        };
        if let Err(e) = native_mods_to_build(&instance.home) {
            self.native_log_preflight(&e);
            self.native_error = Some(e);
            return;
        }
        // Not prepared: the window prepares the base as Prepare & play does, never the command
        // line's hint (#18659). The build keeps this instance and base while it waits.
        let waiting = Waiting {
            needs: backend::prepared_in(&base),
            snapshot: self.native_snapshot(&instance, base, back_up),
            started: false,
        };
        self.native_go(waiting, None, None);
    }

    /// A build waiting for a preparation looks again: one just ended (the version it prepared
    /// and its error as the window shows it), or an install did.
    pub(super) fn native_look_again(&mut self, ended: Option<&str>, error: Option<&str>) {
        if let Some(waiting) = self.native_prepare_first.take() {
            self.native_go(waiting, ended, error);
        }
    }

    /// Builds `waiting`, prepares its base first, waits, or drops it ([`prepare_first::next`]).
    fn native_go(&mut self, mut waiting: Waiting, ended: Option<&str>, error: Option<&str>) {
        // One preparation runs at a time: once one ended, none of this build's runs.
        if ended.is_some() {
            waiting.started = false;
        }
        let instance = instances::list()
            .into_iter()
            .find(|i| i.id == waiting.snapshot.instance);
        let (base, back_up) = (&waiting.snapshot.base, waiting.snapshot.back_up);
        let now = prepare_first::Now {
            snapshot: instance
                .as_ref()
                .map(|i| self.native_snapshot(i, base.clone(), back_up)),
            preparing: self.preparing,
            installing: self.installing.as_ref().map(|(tag, _)| tag.as_str()),
            ended: ended.map(|tag| (tag, error)),
        };
        match prepare_first::next(&waiting, &now, backend::ready) {
            Next::Build => match instance {
                Some(instance) => {
                    self.native_step = None;
                    let Snapshot { base, back_up, .. } = waiting.snapshot;
                    self.start_native_build(instance, base, back_up);
                }
                // `next` builds only an instance it found.
                None => self.native_step = None,
            },
            Next::Prepare => {
                let needs = waiting.needs.clone();
                let started = self.prepare_version(needs.clone(), backend::MinecraftFrom::Install);
                waiting.started = true;
                self.wait_for_prepare(waiting);
                // Refused before a preparation ran (its base was deleted): no preparation will
                // end to wake the build, so it reads the refusal as a preparation that failed.
                if !started {
                    let error = self.prepare_error.clone();
                    self.native_look_again(Some(&needs), error.as_deref());
                }
            }
            Next::Wait => self.wait_for_prepare(waiting),
            Next::Stop(error) => {
                self.native_step = None;
                self.play_after_build = None;
                self.native_log_preflight(&error);
                self.native_error = Some(error);
            }
        }
    }

    /// What a build of `instance` on `base` depends on now ([`Snapshot`]).
    fn native_snapshot(
        &self,
        instance: &instances::Instance,
        base: String,
        back_up: bool,
    ) -> Snapshot {
        let plays = self.version_of(instance).cloned();
        let consent = self.settings.native_mods_allowed_at.is_some();
        Snapshot::of(instance, plays, base, back_up, consent)
    }

    /// The build card shows Prepare's first words while the build waits for it.
    fn wait_for_prepare(&mut self, waiting: Waiting) {
        self.native_prepare_first = Some(waiting);
        self.native_step = Some(tr("Getting ready…").into());
        self.native_progress = None;
    }

    /// Starts building `instance`'s native mods on `base`, on a thread; `Msg::Native` ends it.
    fn start_native_build(&mut self, instance: instances::Instance, base: String, back_up: bool) {
        // The mods this build is given: its failure is read against them, even if another
        // instance is selected by the time it ends.
        self.native_built_mods = native_status::enabled(&instance.home)
            .into_iter()
            .map(|(id, ..)| id)
            .collect();
        // Cleared here, on the window's thread before Stop shows, and not by the worker: a Stop
        // pressed before the worker runs must still end this build.
        native_mods::arm_cancel();
        self.native_step = Some(tr("Starting the build").into());
        self.native_stopping = false;
        self.native_progress = None;
        // The first build downloads the compiler: the card says so, with its size.
        self.first_build = horizon_launcher::rust_toolchain::installed()
            .is_none()
            .then(|| horizon_launcher::rust_toolchain::host().map(|h| h.download_bytes()))
            .flatten();
        self.native_done = None;
        let tx = self.tx.clone();
        let done = self.tx.clone();
        // The worker builds the instance and base the request captured, staging its mods into
        // the shared folder only once it has its turn and lease. The result goes to the window
        // as soon as the build ends, before the old-build cleanup that follows on this thread;
        // a panic still sends one (#18669).
        std::thread::spawn(move || {
            native_mods::deliver_once(
                |deliver| {
                    native_mods::build_then(
                        &base,
                        || native_mods_to_build(&instance.home),
                        true,
                        &instance,
                        None,
                        native_mods::Asked {
                            caller: native_mods::Caller::Window,
                            back_up,
                        },
                        |step| {
                            let _ = tx.send(Msg::NativeProgress(step.shown(), step.progress()));
                        },
                        |inventory| {
                            let (reply, answer) = std::sync::mpsc::channel();
                            tx.send(Msg::NativeRegistryConsent(inventory.clone(), reply))
                                .is_ok()
                                && answer.recv().unwrap_or(false)
                        },
                        deliver,
                    )
                },
                |result| {
                    let _ = done.send(Msg::Native(result));
                },
            );
        });
    }

    /// The build card's Stop: the build under way, or one waiting for a preparation. A
    /// preparation this build started stops too, as Home's Cancel stops it; one the player
    /// started goes on.
    pub(super) fn stop_native_build(&mut self) {
        self.play_after_build = None;
        match self.native_prepare_first.take() {
            Some(waiting) => {
                if waiting.started {
                    horizon_launcher::mojang::CANCEL.store(true, Ordering::Relaxed);
                }
                self.native_step = None;
                self.native_done = Some(tr(native_mods::CANCELLED).into());
            }
            // A download or unpack under way can take a moment to see the stop: the card says
            // so at once, and the build's later steps don't replace it.
            None if self.native_step.is_some() => {
                native_mods::cancel();
                self.native_stopping = true;
                self.native_step = Some(tr("Stopping the build…").into());
            }
            None => native_mods::cancel(),
        }
    }
}

/// The native mods folder of the instance at `home`, refused when its mods can't be read.
fn native_mods_to_build(home: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let folder = native_mods::installed_mods(home)?;
    native_mods::bundles(&folder)?;
    Ok(folder)
}

/// What a failed build says, and the mods that stopped it: a mod's own compile failure, or its
/// build script or features the prebuilt game refuses, in plain words naming it, read against
/// `built`, the mods the build was given (never the instance selected when it ends); any other
/// failure as it is.
fn failure_message(error: String, built: &[String]) -> (String, Vec<String>) {
    if let Some(id) = native_mods::compile_failed_mod(&error, built) {
        return (native_mods::fit::compile_failed(id), vec![id.clone()]);
    }
    match native_mods::refused_mod(&error, built) {
        Some((id, refusal)) => (refusal.plain(), vec![id.clone()]),
        None => (error, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_after_switching_instances_is_read_against_the_mods_built() {
        let error = native_mods::failure(
            &native_mods::compiling_mod("fast-ore"),
            "exit status: 101",
            &["error[E0425]: cannot find value `speed` in this scope".into()],
            &["error: could not compile `fast-ore` (lib) due to 1 previous error".into()],
        );
        // The build was given fast-ore: its failure names it, whatever is selected now.
        let built = vec!["fast-ore".to_string(), "greeter".to_string()];
        let (text, failed) = failure_message(error.clone(), &built);
        assert_eq!(text, native_mods::fit::compile_failed(&built[0]));
        assert_eq!(failed, ["fast-ore"]);
        // Read against another instance's mods it could not be attributed.
        let (text, failed) = failure_message(error.clone(), &["maps".to_string()]);
        assert_eq!(text, error);
        assert!(failed.is_empty());
    }

    #[test]
    fn a_refused_build_script_or_features_reads_like_a_mod_that_failed() {
        let built = vec!["maps".to_string(), "fast-ore".to_string()];
        for refusal in [
            native_mods::Refusal::BuildScript {
                mod_id: "fast-ore".into(),
            },
            native_mods::Refusal::Features {
                mod_id: "fast-ore".into(),
            },
        ] {
            // As the build ends: the log's line, after any mods left out before it.
            let error = format!("maps was left out\n\n{refusal}");
            let (text, failed) = failure_message(error, &built);
            assert_eq!(text, refusal.plain());
            assert_eq!(failed, ["fast-ore"]);
        }
    }
}
