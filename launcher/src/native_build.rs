use super::*;

impl Launcher {
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
                    let _ = tx.send(Msg::NativeProgress(step.label(), step.progress()));
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
