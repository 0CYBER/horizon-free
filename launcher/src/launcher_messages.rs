//! The backend's messages, drained every frame (`Launcher::drain`).
use super::*;

impl Launcher {
    pub(crate) fn drain(&mut self, window: gpui::AnyWindowHandle, cx: &mut Context<Self>) {
        let mut changed = false;
        // A game said which shader API it serves: the packs judged by the launcher's own line
        // while it was asked are judged again.
        let answers = horizon_launcher::game_shader_api::answers();
        if answers != self.shader_answers {
            self.shader_answers = answers;
            self.game_packs.stale();
            changed = true;
        }
        self.perf.tick();
        match self.bench.as_mut().map(|b| b.tick(&self.perf)) {
            Some(perf::Step::Go(page)) => {
                let pages: Vec<Page> = (Page::ALL.iter().map(|p| p.0))
                    .filter(|p| !matches!(p, Page::Wardrobe | Page::Shop) || self.wardrobe_shown())
                    .collect();
                self.go(pages[page % pages.len()]);
                changed = true;
            }
            Some(perf::Step::Scroll) => {
                self.go(Page::Versions);
                changed = true;
            }
            Some(perf::Step::Quit) => cx.quit(),
            Some(perf::Step::Wait) | None => {}
        }
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            match msg {
                Msg::Releases(r) => {
                    if let (Ok(_), Some(checked)) = (&r, self.checked) {
                        self.perf.record_since("launcher.release_list_ms", checked);
                    }
                    if let Ok(list) = &r {
                        horizon_launcher::nightly::note_published(list);
                        // Nightlies of one day are ordered by when they came out, known now.
                        self.installed = backend::installed();
                    }
                    self.releases = r.map_or_else(Load::Failed, Load::Ready);
                    self.notes.clear();
                }
                Msg::ReleasesFromNetwork(list) => {
                    horizon_launcher::nightly::note_published(&list);
                    self.installed = backend::installed();
                    self.stable_failed = None;
                    self.releases = Load::Ready(list);
                    self.notes.clear();
                }
                Msg::StableRetry(next) => {
                    // The same download under its new build: it stays the instance's, and keeps
                    // the version picked on Versions.
                    if self.stable_install.take().is_some() {
                        self.instance_install = Some(next.tag.clone());
                        self.stable_install = Some(next);
                    }
                }
                Msg::Links(links) => {
                    // The official server may have moved: its list entry and its count follow.
                    self.multiplayer.stale();
                    telemetry::set_official_server(&links.official_server);
                    self.links = links;
                    self.follow_min_launcher(cx);
                }
                // The page reads is_prepared when drawn; `changed` redraws it.
                Msg::Reused(_) => self.installed = backend::installed(),
                Msg::Keychain(said) => self.keychain = said,
                Msg::Owner(owns) => self.owner = Some(owns),
                Msg::AccountHead(uuid, url, face, failed) => {
                    self.profile_heads.loaded(&uuid, &url, face, failed)
                }
                Msg::AccountProgress(uuid, generation, phase) => {
                    if self.auth_busy.get(&uuid) == Some(&generation) {
                        self.auth_progress.insert(uuid, phase);
                    }
                }
                Msg::AccountStatus(uuid, generation, status) => {
                    if self.auth_busy.get(&uuid) != Some(&generation) {
                        continue;
                    }
                    self.auth_busy.remove(&uuid);
                    self.auth_progress.remove(&uuid);
                    if self.accounts.find(&uuid).is_some() {
                        if status.state == account::session::State::Ready {
                            self.owner = Some(true);
                        }
                        self.auth_status.insert(uuid, status);
                        // A refresh may have saved a new name or skin: show it now.
                        let saved = Accounts::load();
                        if saved != self.accounts {
                            self.accounts = saved;
                            self.load_profile_heads();
                        }
                    }
                }
                Msg::AccountRemoved(uuid, generation, result) => {
                    if self.auth_busy.get(&uuid) != Some(&generation) {
                        continue;
                    }
                    self.auth_busy.remove(&uuid);
                    self.auth_progress.remove(&uuid);
                    match result {
                        Ok(owns) => {
                            self.accounts = Accounts::load();
                            self.auth_status.remove(&uuid);
                            self.owner = Some(owns);
                        }
                        Err(e) => self.sign_in_state = Some(SignInState::Done(Err(e))),
                    }
                }
                Msg::Playing(playing) => self.playing = playing,
                Msg::OfficialPlayers(players) => self.official_players = players,
                Msg::JavaGame(id, home, cmd) => {
                    self.starting_client = None;
                    self.starting_instance = None;
                    let stop = self.pre_launch_stop.take();
                    let cmd = horizon_launcher::launch::unless_stopped(cmd, stop.as_deref());
                    let spawned = cmd.and_then(|(cmd, vars)| {
                        self.client_vars = vars;
                        backend::spawn(Job::Client, cmd, self.tx.clone())
                    });
                    match spawned {
                        Ok(r) => {
                            self.client = Some(r);
                            self.client_home = Some(home);
                            self.client_run = Some((id, Instant::now()));
                            self.client_error = account::take_unsaved();
                            self.step_aside(cx);
                        }
                        Err(e) => {
                            if e == account::NoOwner::default().to_string() {
                                self.owner = Some(false);
                                self.owner_screen_open = true;
                                self.client_error = Some(account::NoOwner::default().in_window());
                            } else {
                                self.client_error = Some(e);
                            }
                        }
                    }
                }
                Msg::Session(session) => self.handle_game_session(session),
                Msg::PlayStep(step) => {
                    if let Some(line) = self.play_step.said(self.starting_client.is_some(), step) {
                        console_page::keep_line(
                            &mut self.prepare_log,
                            &mut self.log_revision,
                            line,
                        );
                    }
                }
                Msg::JarConsent { jars, reply } => jar_launch::consent(window, jars, reply, cx),
                Msg::JarClient { run, prepared } => {
                    self.starting_client = None;
                    self.starting_instance = None;
                    let stop = self.pre_launch_stop.take();
                    match horizon_launcher::launch::unless_stopped(*prepared, stop.as_deref()) {
                        Ok(cmd) => {
                            self.client_home = cmd.get_current_dir().map(|d| d.to_path_buf());
                            match backend::spawn(Job::Client, cmd, self.tx.clone()) {
                                Ok(r) => {
                                    self.client = Some(r);
                                    self.client_run = run.map(|id| (id, Instant::now()));
                                    self.step_aside(cx);
                                }
                                Err(e) => {
                                    self.client_error = Some(e);
                                    account::discard_unread_sessions();
                                }
                            }
                        }
                        // A start that does not happen leaves no session file behind.
                        Err(e) => {
                            self.client_error = Some(e);
                            account::discard_unread_sessions();
                        }
                    }
                }
                Msg::Progress(tag, p) => {
                    self.bar = (self.bar.1, p);
                    self.installing = Some((tag, p));
                }
                Msg::Installed(tag, result) => {
                    self.installing = None;
                    self.bar = (0., 0.);
                    self.installed = backend::installed();
                    let as_update = self.update_install.take().filter(|t| *t == tag);
                    let for_instance = self.instance_install.take_if(|t| *t == tag).is_some();
                    let stable = self.stable_install.take_if(|r| r.tag == tag);
                    if let (Ok(()), Some(started)) = (&result, self.install_started.take()) {
                        let name = if as_update.is_some() {
                            "launcher.update_ms"
                        } else {
                            "launcher.install_ms"
                        };
                        self.perf.record_since(name, started);
                    }
                    match result {
                        Ok(()) => {
                            if as_update.is_some()
                                && horizon_launcher::nightly::date_of(&tag).is_some()
                            {
                                if let Err(e) = backend::mark_auto_installed(&tag) {
                                    eprintln!("Could not mark updated Nightly {tag}: {e}");
                                }
                            }
                            if self.updates.horizon.as_ref().is_some_and(|r| r.tag == tag) {
                                self.updates.horizon = None;
                            }
                            let before: Vec<String> = self
                                .installed
                                .iter()
                                .filter(|t| **t != tag)
                                .cloned()
                                .collect();
                            // Judged on the selection as saved now, not when the download
                            // started: a version picked on Versions or with `settings
                            // selected-version` meanwhile stays picked. A file from before
                            // `following` is read against the list before this install.
                            // A saved selection that isn't installed is the one the window
                            // replaced at start: the window's stays then.
                            let saved = Settings::load_for_install(&before);
                            if self.settings_save_error.is_none()
                                && saved
                                    .selected
                                    .as_ref()
                                    .is_some_and(|s| self.installed.contains(s))
                            {
                                self.settings.selection_from(&saved);
                            }
                            // An instance's own version leaves the selection where it is, unless
                            // an instance on the Nightly channel is pinned to it and the
                            // selection follows the newest. Home's Stable update never moves it.
                            let instance = (for_instance || stable.is_some()).then(|| {
                                stable.is_none()
                                    && horizon_launcher::instances::list().iter().any(|i| {
                                        i.java.is_none()
                                            && i.channel
                                                == horizon_launcher::instances::Channel::Nightly
                                            && i.version.as_deref() == Some(tag.as_str())
                                    })
                            });
                            self.settings.apply_install(
                                &tag,
                                &before,
                                &self.installed,
                                backend::InstallCause {
                                    update: as_update.is_some(),
                                    instance,
                                },
                            );
                            // Pruning reads settings from disk: protect the selection only once
                            // it has been saved there. A failed save leaves all versions in place.
                            if self.save_settings() {
                                let tx = self.tx.clone();
                                std::thread::spawn(move || {
                                    let _ = tx.send(Msg::Pruned(backend::prune_old_versions()));
                                });
                            }
                        }
                        Err(e) => {
                            // Home's Stable update already read the list again and retried
                            // once: this build isn't offered again until the list is read
                            // from the network, not from a kept copy.
                            self.stable_failed = stable.or(self.stable_failed.take());
                            self.install_error = Some((tag, e));
                        }
                    }
                    if let Some(next) = self.install_next.take() {
                        self.install_for_instance(next);
                    }
                    // After the selection and the queued install: a build waiting for this
                    // install sees both.
                    self.native_look_again(None, None);
                }
                Msg::Removed(tag, result) => {
                    self.removing.retain(|t| *t != tag);
                    self.installed = backend::installed();
                    if self.settings_save_error.is_none() {
                        self.settings.selection_from(&Settings::load());
                    }
                    self.settings.keep_installed_selection(&self.installed);
                    if let Err(e) = result {
                        self.install_error = Some((tag, e));
                    }
                }
                Msg::DiskUse(disk) => self.disk = disk,
                Msg::Storage(result) => self.storage_arrived(result),
                Msg::Imported(result) => {
                    self.importing_build = false;
                    self.installed = backend::installed();
                    match result {
                        // `build_import::import` saved the selection; this keeps the window's copy.
                        Ok(tag) => self.settings.pick(tag),
                        Err(e) => self.import_error = Some(e),
                    }
                }
                Msg::Pruned(gone) => {
                    if !gone.is_empty() {
                        self.installed = backend::installed();
                    }
                }
                Msg::Updates(Ok(updates)) => {
                    self.update_error = None;
                    self.updates = *updates;
                    if self.settings.auto_install() {
                        self.install_update();
                    }
                    if let Some(release) = self.updates.launcher.clone() {
                        self.update_launcher(release);
                    }
                }
                Msg::Updates(Err(e)) => self.update_error = Some(e),
                Msg::NativeRegistryConsent(inventory, reply) => {
                    native_registry_consent::show(window, inventory, reply, cx)
                }
                Msg::NativeProgress(step, progress) => {
                    if !self.native_stopping {
                        self.native_step = Some(step);
                        self.native_progress = Some(progress);
                    }
                }
                Msg::Native(result) => {
                    // The last step of a build that worked says how it went.
                    let stopping = std::mem::take(&mut self.native_stopping);
                    let last = self.native_step.take().filter(|_| !stopping);
                    self.native_progress = None;
                    self.first_build = None;
                    // Build and play: the game starts once the build worked, and only for the
                    // instance it was pressed for, still on the build just made (another may
                    // have been picked, or its version changed, meanwhile).
                    let wanted = self.play_after_build.take();
                    let built = result.as_ref().ok().cloned();
                    let cancelled = matches!(&result, Err(native_mods::Failed::Other(error)) if error == native_mods::CANCELLED);
                    self.native_done = match (&result, cancelled) {
                        (Ok(_), _) => last,
                        (Err(_), true) => Some(tr(native_mods::CANCELLED).into()),
                        (Err(_), false) => None,
                    };
                    self.installed = backend::installed();
                    self.reload_instances();
                    // A build attempt ended: an open panel reads the log it left.
                    self.native_log.stale = true;
                    // No permission: the question is the answer, not the refusal's words.
                    let no_consent = matches!(result, Err(native_mods::Failed::NoConsent));
                    if no_consent {
                        self.request_native_build_consent();
                    }
                    if let (Err(e), false) = (result, cancelled || no_consent) {
                        if let native_mods::Failed::Worlds(stake) = &e {
                            self.keep_stop(stake);
                        }
                        if let native_mods::Failed::Mods { left, .. } = &e {
                            self.native_failed_mods = left.iter().map(|l| l.id.clone()).collect();
                        }
                        // Read against the mods this build was given, not the instance now
                        // selected.
                        let built = std::mem::take(&mut self.native_built_mods);
                        self.native_failed(e.said(native_mods::Caller::Window), &built);
                    }
                    let play_now = wanted.is_some()
                        && built.is_some()
                        && self
                            .selected_instance()
                            .is_some_and(|i| Some(&i.id) == wanted.as_ref() && i.version == built);
                    if play_now {
                        self.play();
                    }
                }
                Msg::LauncherProgress(p) => {
                    if let SelfUpdate::Downloading(_, f) = &mut self.self_update {
                        *f = p;
                    }
                }
                Msg::LauncherUpdated(result) => {
                    let version = match &self.self_update {
                        SelfUpdate::Downloading(v, _) => v.clone(),
                        _ => String::new(),
                    };
                    self.self_update = match result {
                        Ok(staged) => {
                            horizon_launcher::update_handoff::set_pending(staged);
                            SelfUpdate::Ready(version)
                        }
                        Err(e) => SelfUpdate::Failed(e),
                    };
                    self.follow_min_launcher(cx);
                }
                Msg::PackLine(line) => {
                    self.pack_log.push_back(line);
                    if self.pack_log.len() > LOG_LINES {
                        self.pack_log.pop_front();
                    }
                }
                Msg::PackDone(result) => self.pack_done(result),
                Msg::ImportedPacks(asked, packs) => self.packs_read(asked, packs),
                Msg::FoundPacks(found) => {
                    self.found_scanning = false;
                    self.found_packs = found;
                }
                // Old versions log their session; it never reaches the window.
                Msg::Line(_, line) if line.contains("Session ID is") => {}
                Msg::Line(job, line) => {
                    if job == Job::Client && line.starts_with(perf::GAME_FIRST_FRAME) {
                        if let Some(pressed) = self.play_pressed.take() {
                            self.perf
                                .record_since("launcher.play_to_game_frame_ms", pressed);
                        }
                    }
                    // The MCP server keeps its own server's log; the launcher's are the preparer's
                    // and the game's.
                    if job != Job::Server {
                        console_page::keep_line(
                            &mut self.prepare_log,
                            &mut self.log_revision,
                            line,
                        );
                    }
                }
                Msg::Exit(job, result) => {
                    // A preparation (or a build) ended: the disk may have changed under the pages.
                    backend::forget_prepared();
                    let error = result.err();
                    match job {
                        Job::Prepare => {
                            self.preparing = false;
                            let tag = self.prepare_tag.take();
                            let error =
                                prepare_outcome(error, tag.as_deref(), backend::check_prepared);
                            // A Play waiting for this preparation starts below once nothing
                            // else is starting; one it failed for cannot.
                            if error.is_some() {
                                self.play_when_prepared = None;
                            }
                            if let (None, Some(started)) = (&error, self.prepare_started.take()) {
                                self.perf.record_since("launcher.prepare_ms", started);
                            }
                            // A preparer process's exit status says little, so its last line
                            // follows; the launcher's own errors are full sentences already.
                            self.prepare_error = error.map(|e| {
                                let tail = self
                                    .prepare_log
                                    .iter()
                                    .rev()
                                    .find(|l| !l.trim().is_empty())
                                    .filter(|_| e.starts_with("Exited with"));
                                tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                            });
                            self.prepare_fix = prepare_fix_after(
                                self.prepare_error.is_some(),
                                backend::prepare_offers_mojang(),
                                tag.as_deref(),
                            );
                            // Deleted while it was being prepared: said as Prepare says it.
                            if let Some(tag) =
                                tag.as_deref().filter(|_| self.prepare_error.is_some())
                            {
                                self.version_still_installed(tag);
                            }
                            let (ended, error) = (
                                tag.map(|t| backend::prepared_in(&t)),
                                self.prepare_error.clone(),
                            );
                            self.native_look_again(ended.as_deref(), error.as_deref());
                        }
                        Job::Client => {
                            // The exit thread reaped the game; its child keeps the status.
                            let client = self.client.take();
                            let leases_refused = client
                                .as_ref()
                                .is_some_and(|client| client.leases_refused.load(Ordering::SeqCst));
                            let native_crash = client
                                .and_then(|client| client.child.lock().ok()?.try_wait().ok()?)
                                .is_some_and(backend::native_crash_exit);
                            // Any session file the game didn't read (it crashed first).
                            account::discard_unread_sessions();
                            self.come_back(cx);
                            // The game saved: last played, sizes and pictures changed.
                            self.worlds.stale();
                            self.multiplayer.stale();
                            self.screenshots.stale();
                            self.game_packs.stale();
                            let vars = std::mem::take(&mut self.client_vars);
                            if let Some((id, started)) = self.client_run.take() {
                                if let Err(e) = instances::record_play(&id, started.elapsed()) {
                                    eprintln!("Play time not saved: {e}");
                                }
                                if let Some(i) = instances::list().into_iter().find(|i| i.id == id)
                                {
                                    horizon_launcher::launch::run_after_exit(
                                        &i.launch, &i.home, &vars,
                                    );
                                }
                                self.reload_instances();
                            }
                            // The run's own game folder: an instance's or a pack's keeps its reports.
                            let home = self.client_home.clone().unwrap_or_else(backend::game_home);
                            let since = self.client_started.take();
                            let report =
                                horizon_launcher::refusals::crash_report(leases_refused, || {
                                    since
                                        .and_then(|since| backend::crash_report_since(&home, since))
                                });
                            let switched = if native_crash && report.is_none() {
                                self.switched_after_start_crash()
                            } else {
                                None
                            };
                            if let Some((crashed, api)) = switched {
                                self.client_error = Some(trf(
                                    "Horizon's {crashed} renderer crashed while starting on this \
                                     graphics card, so the game now uses {api}. Press Play to \
                                     start it again. Video Settings > Graphics API switches back.",
                                    &[("crashed", crashed), ("api", api)],
                                ));
                            } else if error.is_some() || report.is_some() {
                                let summary = report
                                    .as_ref()
                                    .and_then(|p| std::fs::read_to_string(p).ok())
                                    .and_then(|text| backend::crash_summary(&text))
                                    .or_else(|| {
                                        let tail = self
                                            .prepare_log
                                            .iter()
                                            .rev()
                                            .find(|l| !l.trim().is_empty());
                                        // A full sentence (a lease refusal) reads on into the line.
                                        error.as_ref().map(|e| {
                                            let gap = if e.ends_with('.') { " " } else { ": " };
                                            tail.map_or(e.clone(), |t| format!("{e}{gap}{t}"))
                                        })
                                    })
                                    .unwrap_or_default();
                                self.crash = Some(Crash {
                                    report,
                                    summary,
                                    copied: false,
                                    leases_refused,
                                });
                            }
                        }
                        Job::Server => {}
                    }
                }
            }
        }
        // A start that ended (the game started, or it failed) is on no step.
        self.play_step.settle(self.starting_client.is_some());
        changed |= self.servers_tick();
        changed |= self.mods_tick(window, cx);
        changed |= self.github_tick();
        changed |= self.native_log_read(cx);
        changed |= self.toast_tick();
        if let Some(sign_in) = &self.sign_in {
            if let Some(state) = sign_in_change(&mut self.sign_in_state, sign_in.state()) {
                if let SignInState::Done(Ok(ref account)) = state {
                    self.auth_busy.remove(&account.uuid);
                    self.sign_in_target = None;
                    self.accounts = Accounts::load();
                    self.load_profile_heads();
                    self.load_auth_statuses(false);
                    // Sign-in refuses accounts that don't own the game.
                    self.owner = Some(true);
                    self.owner_screen_open = false;
                    self.client_error = None;
                    if !self.settings.owner_notice_seen {
                        self.settings.owner_notice_seen = true;
                        self.save_settings();
                    }
                }
                changed = true;
            }
        }
        if self
            .checked
            .is_some_and(|at| at.elapsed() > backend::CHECK_EVERY)
        {
            self.check_updates();
        }
        // A Play that waited for its version, or for another start (a pre-launch command's
        // Stop among it) to finish, starts as soon as nothing else is starting.
        self.start_queued_play();
        if changed {
            cx.notify();
        }
    }
}

/// What a preparation for `tag` that ended with `error` says: its error, or, when it ended
/// without one but `ready` finds `tag` still not prepared, that, rather than nothing.
fn prepare_outcome(
    error: Option<String>,
    tag: Option<&str>,
    ready: impl Fn(&str) -> bool,
) -> Option<String> {
    error.or_else(|| tag.filter(|tag| !ready(tag)).map(not_prepared_after))
}

/// What the error of the preparation of `tag` offers: the download from Mojang when the folders
/// lacked a library ([`backend::prepare_offers_mojang`]), for that version even if another is
/// selected by the time it is pressed.
fn prepare_fix_after(failed: bool, offers_mojang: bool, tag: Option<&str>) -> Option<PrepareFix> {
    tag.filter(|_| failed && offers_mojang)
        .map(|tag| PrepareFix::Mojang(tag.to_owned()))
}

/// What Home's Prepare notice says, and the action it offers. The selected instance's pinned
/// version that is missing (`pin`) comes first: Play refuses it, so an older error about another
/// version, with that version's action, would point the player the wrong way. Then the last
/// preparation's `error` with its `fix`; `gone` words the missing version.
pub(crate) fn prepare_notice_choice(
    pin: Option<&str>,
    error: Option<String>,
    fix: Option<PrepareFix>,
    gone: impl Fn(&str) -> String,
) -> Option<(String, Option<PrepareFix>)> {
    match (pin, error) {
        (Some(pin), _) => Some((gone(pin), Some(PrepareFix::Reinstall(pin.to_owned())))),
        (None, Some(error)) => Some((error, fix)),
        (None, None) => None,
    }
}

/// What Home says when a preparation ended without an error but `tag` still isn't prepared: the
/// game can't start, and the button alone would only offer the same step again.
fn not_prepared_after(tag: &str) -> String {
    crate::i18n::trf(
        "The preparation of {version} ended, but its files are still not ready, so the game can't start.",
        &[("version", tag)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mojang_download_offer_keeps_the_version_that_failed() {
        assert_eq!(
            prepare_fix_after(true, true, Some("v-a")),
            Some(PrepareFix::Mojang("v-a".into()))
        );
        assert_eq!(prepare_fix_after(true, false, Some("v-a")), None);
        assert_eq!(prepare_fix_after(false, true, Some("v-a")), None);
    }

    /// Prepare failed for A, then an instance pinned to the missing B was selected: B's notice
    /// and Reinstall, never A's Mojang retry.
    #[test]
    fn a_missing_pin_outranks_an_older_prepare_error() {
        let gone = |tag: &str| format!("{tag} is gone");
        let a_failed = || Some("A's files are incomplete".to_string());
        let a_fix = || Some(PrepareFix::Mojang("v-a".into()));
        assert_eq!(
            prepare_notice_choice(Some("v-b"), a_failed(), a_fix(), gone),
            Some((
                "v-b is gone".into(),
                Some(PrepareFix::Reinstall("v-b".into()))
            ))
        );
        assert_eq!(
            prepare_notice_choice(None, a_failed(), a_fix(), gone),
            Some(("A's files are incomplete".into(), a_fix()))
        );
        assert_eq!(prepare_notice_choice(None, None, None, gone), None);
    }

    #[test]
    fn a_preparation_that_leaves_its_version_unprepared_says_so() {
        let failed = Some("Exited with 1".to_string());
        assert_eq!(
            prepare_outcome(failed.clone(), Some("v1"), |_| false),
            failed
        );
        assert_eq!(prepare_outcome(None, Some("v1"), |_| true), None);
        assert_eq!(prepare_outcome(None, None, |_| false), None);
        assert_eq!(
            prepare_outcome(None, Some("v1"), |_| false),
            Some(not_prepared_after("v1"))
        );
    }
}
