use super::*;

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
        // The mods this build is given: its failure is read against them, even if another
        // instance is selected by the time it ends.
        self.native_built_mods = native_status::enabled(&self.mods.home())
            .into_iter()
            .map(|(id, ..)| id)
            .collect();
        let folder = match self.native_mods_to_build() {
            Ok((folder, _)) => folder,
            Err(e) => {
                self.native_log_preflight(&e);
                self.native_error = Some(e);
                return;
            }
        };
        self.native_step = Some(tr("Starting the build").into());
        self.native_progress = None;
        // The first build downloads the compiler: the card says so, with its size.
        self.first_build = horizon_launcher::rust_toolchain::installed()
            .is_none()
            .then(|| horizon_launcher::rust_toolchain::host().map(|h| h.download_bytes()))
            .flatten();
        self.native_done = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = native_mods::build(
                &base,
                &folder,
                &instance,
                None,
                back_up,
                |step| {
                    let _ = tx.send(Msg::NativeProgress(step.shown(), step.progress()));
                },
                |inventory| {
                    let (reply, answer) = std::sync::mpsc::channel();
                    tx.send(Msg::NativeRegistryConsent(inventory.clone(), reply))
                        .is_ok()
                        && answer.recv().unwrap_or(false)
                },
            );
            let _ = tx.send(Msg::Native(result));
        });
    }
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
