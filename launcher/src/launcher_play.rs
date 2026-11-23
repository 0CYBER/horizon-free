//! Installing and removing builds, preparing and starting the game, and the accounts' sign-in.
use super::*;

impl Launcher {
    /// After a native crash with no report (`backend::native_crash_exit`): when the game's output
    /// ends while its Vulkan or OpenGL renderer started, a crash in the graphics driver
    /// (arismoko/horizon#746, #18291), switches the game to `backend::start_crash_fallback`
    /// (DirectX 12 on Windows, OpenGL after Vulkan on Linux; only where the crashed run left the
    /// graphics API on Auto, `backend::switch_after_start_crash`). The crashed API's label and the
    /// new one's when it did.
    pub(crate) fn switched_after_start_crash(&self) -> Option<(&'static str, &'static str)> {
        let crashed = backend::renderer_start_crash(&self.prepare_log)?;
        let (api, label) = backend::start_crash_fallback(crashed, std::env::consts::OS)?;
        let game_home = backend::game_home();
        let run_home = self
            .client_home
            .clone()
            .unwrap_or_else(|| game_home.clone());
        let build = self.client_build.as_deref().unwrap_or_default();
        match backend::switch_after_start_crash(&run_home, &game_home, api, build) {
            Ok(switched) => switched.then_some((crashed.label(), label)),
            Err(e) => {
                eprintln!("Horizon launcher: could not switch to {label}: {e}");
                None
            }
        }
    }

    /// The version Play starts. A Horizon instance with a missing pin has no playable build.
    pub(crate) fn selected(&self) -> Option<&String> {
        match self.selected_instance().filter(|i| i.java.is_none()) {
            Some(instance) => self.version_of(instance),
            None => self.launcher_version(),
        }
    }

    /// The launcher's default resolution for screens that need a Horizon build without an instance.
    pub(crate) fn launcher_version(&self) -> Option<&String> {
        let version = horizon_launcher::nightly::latest_for(
            instances::Channel::Nightly,
            &self.installed,
            self.settings.selected.as_deref(),
        )?;
        self.installed.iter().find(|tag| **tag == version)
    }

    /// Installs `release` for an instance made or edited to play it, after the install running
    /// now if there is one.
    pub(crate) fn install_for_instance(&mut self, release: Release) {
        let running = self.installing.as_ref().map(|(tag, _)| tag.as_str());
        enqueue_instance_install(&mut self.install_queue, running, release);
        self.start_next_instance_install();
    }

    pub(super) fn start_next_instance_install(&mut self) {
        if self.installing.is_some() {
            return;
        }
        while let Some(release) = take_instance_install(&mut self.install_queue, &self.removing) {
            // Another window or a retried Stable update may have supplied the queued build.
            if backend::is_installed(&release.tag) {
                if !self.installed.contains(&release.tag) {
                    self.installed = backend::installed();
                }
                continue;
            }
            self.instance_install = Some(release.tag.clone());
            self.start_install(&release, None);
            return;
        }
    }

    pub(crate) fn install(&mut self, release: &Release) {
        let id = self
            .selected_instance()
            .map_or_else(|| instances::DEFAULT.into(), |i| i.id.clone());
        self.start_install(release, Some(id));
    }

    pub(super) fn start_install(&mut self, release: &Release, manual_target: Option<String>) {
        if release.asset.is_none()
            || self.installing.is_some()
            || self.removing.contains(&release.tag)
            || self
                .install_queue
                .iter()
                .any(|queued| queued.tag == release.tag)
        {
            return;
        }
        let manual_target = match manual_target
            .map(instances::manual_install_target)
            .transpose()
        {
            Ok(target) => target,
            Err(e) => {
                self.install_error = Some((release.tag.clone(), e));
                return;
            }
        };
        let (release, tx) = (release.clone(), self.tx.clone());
        let tag = release.tag.clone();
        self.installing = Some((tag.clone(), 0.0));
        self.install_started = Some(Instant::now());
        self.bar = (0., 0.);
        self.manual_install_for = manual_target;
        clear_install_error_for(&mut self.install_error, &tag);
        let stable = self
            .stable_install
            .as_ref()
            .is_some_and(|r| r.tag == release.tag);
        std::thread::spawn(move || {
            let install = |r: &Release| {
                backend::install(r, |p| {
                    let _ = tx.send(Msg::Progress(r.tag.clone(), p));
                })
            };
            if !stable {
                let result = install(&release);
                let _ = tx.send(Msg::Installed(tag, result));
                return;
            }
            // Home's Stable update: one press, even when `nightly` moved on since the offer.
            let (done, result) = horizon_launcher::nightly::install_stable(
                release,
                backend::releases_from_network,
                |list, next| {
                    let _ = tx.send(Msg::ReleasesFromNetwork(list));
                    if let Some(next) = next {
                        let _ = tx.send(Msg::StableRetry(next.clone()));
                    }
                },
                |tag| backend::installed().iter().any(|t| t == tag),
                install,
            );
            let _ = tx.send(Msg::Installed(done.tag, result));
        });
    }

    /// Deletes an installed version on a background thread: a version is thousands of files and
    /// the window must keep drawing meanwhile. `Msg::Removed` ends it.
    pub(crate) fn remove(&mut self, tag: &str) {
        self.confirm_remove = None;
        let installing = self.installing.as_ref().map(|(t, _)| t.as_str());
        if !removable(
            &self.installed,
            installing,
            &self.removing,
            &self.instances_using(tag),
            tag,
        ) {
            return;
        }
        self.install_error = None;
        self.removing.push(tag.to_string());
        self.installed.retain(|t| t != tag);
        self.settings.keep_installed_selection(&self.installed);
        let (tag, tx) = (tag.to_string(), self.tx.clone());
        std::thread::spawn(move || {
            let result = backend::remove_version(&tag);
            let _ = tx.send(Msg::Removed(tag, result));
        });
    }

    /// Asks for a game program (or a zip holding one) and installs it as its own version on a
    /// thread: a version is thousands of files. `Msg::Imported` ends it.
    pub(crate) fn import_build(&mut self, cx: &mut Context<Self>) {
        if !self.can_import_build() {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("Import").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.pop() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                // Another picker's import, or a removal, may have started meanwhile.
                this.start_build_import(path, cx);
            });
        })
        .detach();
    }
    /// Whether a build may be imported now. A removal still running could take
    /// the folder of the same program imported again.
    pub(crate) fn can_import_build(&self) -> bool {
        !self.importing_build && self.removing.is_empty()
    }
    /// Starts importing the build at `path` on a thread, as the picker does.
    /// Separate from [`Self::import_build`] so a file dropped on the window can
    /// start the same import (`import_dropped`) without a picker. Does nothing
    /// when one is already running. `Msg::Imported` ends it.
    pub(crate) fn start_build_import(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        let Some(instance) = self
            .selected_instance()
            .filter(|_| self.can_import_build())
            .cloned()
        else {
            return;
        };
        self.importing_build = true;
        self.import_error = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Imported(build_import::import_for(
                &path, None, &instance,
            )));
        });
        cx.notify();
    }

    /// Prepares the selected version, its Minecraft files from the player's install or, for an
    /// owner who asks, from Mojang.
    pub(crate) fn prepare(&mut self, from: backend::MinecraftFrom) {
        if let Some(tag) = self.selected().cloned() {
            self.prepare_version(tag, from);
        }
    }

    /// Whether `tag`, and the version whose files it prepares (a native build's base), are still
    /// installed. The window reads the list once and a player may delete `versions` while it is
    /// open: then the list is read again, the selection moves as after a removal, and Home says
    /// so with Reinstall, never the deleted folder's OS error.
    pub(crate) fn version_still_installed(&mut self, tag: &str) -> bool {
        let base = backend::prepared_in(tag);
        let Some((gone, installed)) = backend::gone(&[tag, &base]) else {
            return true;
        };
        self.installed = installed;
        if self.settings_save_error.is_none() {
            self.settings.selection_from(&Settings::load());
        }
        self.settings.keep_installed_selection(&self.installed);
        backend::forget_prepared();
        self.play_when_prepared = None;
        self.prepare_error = Some(backend::version_gone(&self.version_name(&gone)));
        self.prepare_fix = Some(PrepareFix::Reinstall(gone));
        false
    }

    /// Reinstall from the notice of a version that is gone: unless it is back (another launcher or
    /// the command line was only replacing it), then the list is read again and nothing else is
    /// done.
    pub(crate) fn reinstall(&mut self, release: &Release) {
        match backend::installed_again(&release.tag) {
            Some(installed) => {
                self.installed = installed;
                self.settings.keep_installed_selection(&self.installed);
                backend::forget_prepared();
            }
            None => self.install_for_instance(release.clone()),
        }
    }

    /// [`Self::prepare`] for `tag` rather than the selected version. False when it didn't start:
    /// the version is gone, and `prepare_error` says so.
    pub(crate) fn prepare_version(&mut self, tag: String, from: backend::MinecraftFrom) -> bool {
        if !self.version_still_installed(&tag) {
            return false;
        }
        self.prepare_tag = Some(tag.clone());
        horizon_launcher::mojang::CANCEL.store(false, Ordering::Relaxed);
        let minecraft = self
            .settings
            .minecraft_dir
            .clone()
            .unwrap_or_else(|| horizon_platform::prepare::artifacts::minecraft_dir(None));
        self.prepare_log.clear();
        self.prepare_error = None;
        self.prepare_fix = None;
        self.prepare_log
            .push_back("Checking Java and your Minecraft install".into());
        let tx = self.tx.clone();
        // The checks may download the server jar, so they run on the thread too.
        std::thread::spawn(move || {
            let log = |line| {
                let _ = tx.send(Msg::Line(Job::Prepare, line));
            };
            // A native build reads its base's files: those are the ones prepared.
            let tag = backend::prepared_in(&tag);
            let result = backend::preparer(&tag, &minecraft, from, &log)
                .inspect(|(_, minecraft)| backend::sounds(&tag, minecraft, &log))
                .map(|(p, _)| {
                    backend::set_prepare_step(backend::PrepareStep::Prepare);
                    p
                })
                .and_then(|p| match p {
                    backend::Preparer::Process(cmd) => {
                        backend::spawn(Job::Prepare, cmd, tx.clone()).map(|_running| false)
                    }
                    backend::Preparer::InProcess(options) => {
                        backend::prepare_in_process(&options, &log).map(|_| true)
                    }
                });
            match result {
                // A process sends its own Exit when it ends.
                Ok(false) => {}
                Ok(true) => {
                    let _ = tx.send(Msg::Exit(Job::Prepare, Ok(())));
                }
                Err(e) => {
                    let _ = tx.send(Msg::Exit(Job::Prepare, Err(e)));
                }
            }
        });
        self.preparing = true;
        self.prepare_started = Some(Instant::now());
        self.download_seen = None;
        true
    }

    pub(crate) fn play(&mut self) {
        let id = self
            .selected_instance()
            .map_or(instances::DEFAULT, |i| i.id.as_str());
        let instance = match instances::for_launch_id(id) {
            Ok(instance) => instance,
            Err(e) => {
                self.client_error = Some(e);
                return;
            }
        };
        if instance.java.is_some() {
            // Mojang's game: no Horizon version or preparation, only no game already running.
            // It joins a server; a friend's code and opening a world are Horizon's.
            self.joining = None;
            let server = self
                .quick_play
                .take()
                .filter(|(flag, _)| *flag == "--server")
                .map(|(_, ip)| ip);
            if self.client.is_some() || self.starting_client.is_some() {
                return;
            }
            self.starting_pack = None;
            self.start_java(instance, server);
            return;
        }
        let tag = match instance.resolve_version(None) {
            Ok(tag) => tag,
            Err(e) => {
                self.client_error = Some(e);
                return;
            }
        };
        if !self.version_still_installed(&tag) || !self.can_start(&tag) {
            return;
        }
        self.starting_pack = None;
        self.start_client(tag, Some(instance.id));
    }

    /// Starts a Java instance's Minecraft, joining `server` if given: its files and Java first
    /// (downloaded the first time, their lines in the log), then `drain` starts the game when
    /// the command arrives.
    pub(crate) fn start_java(&mut self, instance: instances::Instance, server: Option<String>) {
        self.client_error = None;
        self.crash = None;
        self.prepare_log.clear();
        self.client_started =
            Some(std::time::SystemTime::now() - std::time::Duration::from_secs(2));
        self.starting_client = Some(instance.id.clone());
        self.starting_instance = Some(instance.id.clone());
        let stop = self.pre_launch_stop_for(&instance.launch);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let log = |line: String| {
                let _ = tx.send(Msg::Line(Job::Client, line));
            };
            let cmd = backend::java_game(&instance, server.as_deref(), &log).and_then(|mut cmd| {
                let vars = instance.vars(cmd.get_program());
                cmd.envs(vars.iter().map(|(k, v)| (k, v)));
                let launch = &instance.launch;
                Ok((
                    horizon_launcher::launch::wrap_stoppable(cmd, launch, &stop)?,
                    vars,
                ))
            });
            let _ = tx.send(Msg::JavaGame(instance.id, instance.home, cmd));
        });
    }

    /// Whether the start under way can be stopped for its instance's pre-launch command, which
    /// Windows runs to its end before the game.
    pub(crate) fn pre_launch_stoppable(&self) -> bool {
        self.pre_launch_stop.is_some()
    }

    /// Stops the start under way, its pre-launch command with what it started; once that
    /// succeeded, the game still does not start.
    pub(crate) fn stop_pre_launch(&self) {
        if let Some(stop) = &self.pre_launch_stop {
            stop.store(true, Ordering::Relaxed);
        }
    }

    /// The Stop for a start with `launch`, handed to the start's thread; the window keeps it
    /// ([`Self::pre_launch_stoppable`]) when Windows runs a pre-launch command.
    pub(crate) fn pre_launch_stop_for(
        &mut self,
        launch: &horizon_launcher::launch::Launch,
    ) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        let stop = std::sync::Arc::default();
        self.pre_launch_stop = (cfg!(windows) && !launch.pre_launch.trim().is_empty())
            .then(|| std::sync::Arc::clone(&stop));
        stop
    }

    /// Whether `tag` can start now ([`starts_now`]): prepared, no preparation under way, no game
    /// running or starting.
    pub(crate) fn can_start(&self, tag: &str) -> bool {
        let running = self.client.is_some() || self.starting_client.is_some();
        self.settings.instance_versions_migrated
            && starts_now(backend::is_prepared(tag), self.preparing, running)
    }

    /// Whether the selected instance's version can start now, from a shortcut (a server's Join,
    /// a world's Play, a friend's code): Horizon's only, never a Java instance.
    pub(crate) fn can_start_selected(&self) -> bool {
        self.selected_instance().is_none_or(|i| i.java.is_none())
            && self.selected().is_some_and(|tag| self.can_start(tag))
    }

    /// Starts Play's request after its version was prepared, once nothing else is starting:
    /// a start whose pre-launch command still runs owns the window's Stop (Windows), and the
    /// queued Play waits rather than taking the Stop over.
    pub(crate) fn start_queued_play(&mut self) {
        let start_busy = self.preparing
            || self.pre_launch_stoppable()
            || self.starting_client.is_some()
            || self.client.is_some();
        let Some((tag, instance)) = take_queued_play(&mut self.play_when_prepared, start_busy)
        else {
            return;
        };
        if backend::check_prepared(&tag) {
            self.starting_pack = None;
            self.start_client(tag, Some(instance));
        }
    }

    /// Whether a server's Join starts the selected instance now: a Java instance too, as Play
    /// would (no game running or starting).
    pub(crate) fn can_join_selected(&self) -> bool {
        match self.selected_instance() {
            Some(i) if i.java.is_some() => self.client.is_none() && self.starting_client.is_none(),
            _ => self.can_start_selected(),
        }
    }

    /// Starts the game: the session first (a network round trip when the token needs
    /// renewing), then `drain` starts the process when it arrives.
    pub(crate) fn start_client(&mut self, tag: String, instance_id: Option<String>) {
        // If the migration could not save a pin, starting Latest could upgrade a held world.
        if !self.settings.instance_versions_migrated || !self.version_still_installed(&tag) {
            self.starting_pack = None;
            self.joining = None;
            self.quick_play = None;
            return;
        }
        // The folder this start reads, a pack's carried mods among it: never one being ported.
        let work = self.starting_pack.clone().or_else(|| {
            self.instances
                .list
                .iter()
                .find(|i| Some(&i.id) == instance_id.as_ref())
                .map(|i| i.work.clone())
        });
        if let Some(why) = work.as_deref().and_then(|w| self.pack_folder_busy(w)) {
            self.client_error = Some(tr(why).into());
            self.starting_pack = None;
            self.joining = None;
            self.quick_play = None;
            return;
        }
        self.starting_work = work;
        self.client_error = None;
        self.crash = None;
        self.prepare_log.clear();
        // A little earlier than the spawn: file times can be coarse.
        self.client_started =
            Some(std::time::SystemTime::now() - std::time::Duration::from_secs(2));
        // What the resources lack (built-in packs, panorama, sounds) and the session (a network
        // round trip when the token needs renewing) come first; `drain` starts the game when the
        // session arrives.
        self.starting_client = Some(tag.clone());
        self.client_build = Some(tag.clone());
        self.starting_instance = instance_id.clone();
        self.play_pressed = Some(Instant::now());
        let tx = self.tx.clone();
        let auto = self
            .instances
            .list
            .iter()
            .find(|i| Some(&i.id) == instance_id.as_ref())
            .filter(|_| self.starting_pack.is_none())
            .map(|i| mods_page::AutoTarget {
                instance: i.clone(),
            });
        let mods_tx = self.mods.tx.clone();
        let auto_enabled = self.settings.auto_updates_mods();
        std::thread::spawn(move || {
            // One step at a time, on stderr and in the window.
            let say = |stderr: &str, shown: &str| play_step::say(&tx, stderr, shown.into());
            say(
                "checking the game's assets",
                tr("Checking the game's assets…"),
            );
            backend::complete_assets(&tag);
            if let Some(auto) = auto {
                say("updating registry mods", tr("Updating registry mods…"));
                // The project downloading now, once per project.
                let last = std::cell::RefCell::new(String::new());
                let progress = |project: &str| {
                    if *last.borrow() != project {
                        project.clone_into(&mut last.borrow_mut());
                        let shown =
                            trf("Updating registry mods: {project}", &[("project", project)]);
                        play_step::say(&tx, &format!("updating {project}"), shown);
                    }
                };
                let lines = mods_page::auto_update(&[auto], auto_enabled, &progress);
                if !lines.is_empty() {
                    let _ = mods_tx.send(mods_page::Event::AutoUpdated(lines));
                }
            }
            say("reading the sign-in", tr("Reading the sign-in…"));
            let session = if horizon_launcher::developer_offline::enabled(&tag) {
                Ok(None)
            } else {
                account::session_for_game(&backend::game_home()).map(Some)
            };
            say("starting the game", tr("Starting the game…"));
            let _ = tx.send(Msg::Session(session));
        });
    }

    /// The prepared game files of the selected version (else the first installed): where the
    /// default skins are.
    pub(crate) fn game_resources(&self) -> std::path::PathBuf {
        self.selected()
            .or_else(|| self.installed.first())
            .map(|tag| backend::resources(tag))
            .unwrap_or_default()
    }

    pub(crate) fn load_profile_heads(&mut self) {
        let resources = self.game_resources();
        self.profile_heads
            .request(&self.accounts.accounts, resources, &self.tx);
    }

    pub(crate) fn begin_auth_job(&mut self, uuid: &str) -> Option<u64> {
        if self.auth_busy.contains_key(uuid) {
            return None;
        }
        self.auth_next += 1;
        self.auth_busy.insert(uuid.into(), self.auth_next);
        self.auth_progress
            .insert(uuid.into(), account::session::Phase::Queued);
        Some(self.auth_next)
    }

    /// Only workers read the keychain or contact authentication services. Cached public status
    /// keeps drawing independent of keychain prompts, network timeouts and refresh operations.
    pub(crate) fn load_auth_statuses(&mut self, renew_due: bool) {
        self.auth_checked = Some(Instant::now());
        for account in self.accounts.accounts.clone() {
            let Some(generation) = self.begin_auth_job(&account.uuid) else {
                continue;
            };
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let progress = |phase| {
                    let _ = tx.send(Msg::AccountProgress(
                        account.uuid.clone(),
                        generation,
                        phase,
                    ));
                };
                progress(account::session::Phase::Keychain);
                let store = account::store();
                let mut status =
                    account::session::status_with_progress(store, &account.uuid, &progress);
                if renew_due && status.state == account::session::State::NeedsRefresh {
                    if let Ok(renewed) =
                        account::session::refresh_with_progress(&account.uuid, &progress)
                    {
                        status = renewed;
                    }
                }
                let _ = tx.send(Msg::AccountStatus(account.uuid, generation, status));
            });
        }
    }

    pub(crate) fn refresh_account(&mut self, uuid: String) {
        let Some(generation) = self.begin_auth_job(&uuid) else {
            return;
        };
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let progress = |phase| {
                let _ = tx.send(Msg::AccountProgress(uuid.clone(), generation, phase));
            };
            progress(account::session::Phase::Keychain);
            if let Ok(status) = account::session::refresh_with_progress(&uuid, &progress) {
                let _ = tx.send(Msg::AccountStatus(uuid, generation, status));
            } else {
                // It may have been removed through the CLI while this window was open.
                let _ = tx.send(Msg::AccountRemoved(
                    uuid,
                    generation,
                    Ok(Accounts::load().has_owner(account::store())),
                ));
            }
        });
    }

    /// Generic Add account and targeted re-sign-in share the launcher-owned device-code flow.
    pub(crate) fn start_sign_in(&mut self) {
        self.begin_sign_in(None);
    }

    pub(crate) fn re_sign_in(&mut self, uuid: &str) {
        if let Some(account) = self.accounts.find(uuid).cloned() {
            self.begin_sign_in(Some(account));
        }
    }

    pub(crate) fn begin_sign_in(&mut self, target: Option<account::Account>) {
        self.cancel_sign_in();
        // The code page offers a private window: find the default browser meanwhile.
        std::thread::spawn(|| {
            horizon_launcher::private_window::detected();
        });
        self.sign_in = Some(match &target {
            Some(account) => SignIn::start_for(account.clone()),
            None => SignIn::start(),
        });
        self.sign_in_target = target;
        self.sign_in_state = Some(SignInState::Starting);
    }

    pub(crate) fn cancel_sign_in(&mut self) {
        if let Some(old) = self.sign_in.take() {
            old.cancel();
        }
        self.sign_in_state = None;
        self.sign_in_target = None;
    }

    pub(crate) fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.settings.minecraft_dir = Some(path);
                        this.save_settings();
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }
}

fn clear_install_error_for(error: &mut Option<(String, String)>, tag: &str) {
    error.take_if(|(failed, _)| failed == tag);
}

fn enqueue_instance_install(
    queue: &mut VecDeque<Release>,
    running: Option<&str>,
    release: Release,
) {
    if release.asset.is_some()
        && running != Some(release.tag.as_str())
        && queue.iter().all(|queued| queued.tag != release.tag)
    {
        queue.push_back(release);
    }
}

fn take_instance_install(queue: &mut VecDeque<Release>, removing: &[String]) -> Option<Release> {
    if queue
        .front()
        .is_some_and(|release| removing.contains(&release.tag))
    {
        return None;
    }
    queue.pop_front()
}

/// Takes the Play that waited for its version to be prepared, unless another start is under
/// way: then it stays queued for [`Launcher::start_queued_play`] to try again.
fn take_queued_play(
    queued: &mut Option<(String, String)>,
    start_busy: bool,
) -> Option<(String, String)> {
    if start_busy {
        return None;
    }
    queued.take()
}

#[cfg(test)]
mod tests {
    use super::{
        clear_install_error_for, enqueue_instance_install, take_instance_install, take_queued_play,
        Release, VecDeque,
    };

    fn queued() -> Option<(String, String)> {
        Some(("nightly-2026-11-19".into(), "instance-1".into()))
    }

    #[test]
    fn instance_downloads_wait_in_order_and_keep_each_distinct_save() {
        let release = |tag: &str| Release {
            tag: tag.into(),
            name: tag.into(),
            body: String::new(),
            date: String::new(),
            time: String::new(),
            asset: Some(("fixture".into(), 1)),
            prerelease: false,
            nightly: None,
        };
        let mut queue = VecDeque::new();
        for tag in ["first", "second", "first", "running", "third"] {
            enqueue_instance_install(&mut queue, Some("running"), release(tag));
        }

        assert!(take_instance_install(&mut queue, &["first".into()]).is_none());
        for tag in ["first", "second", "third"] {
            assert_eq!(take_instance_install(&mut queue, &[]).unwrap().tag, tag);
        }
        assert!(take_instance_install(&mut queue, &[]).is_none());
    }

    #[test]
    fn starting_a_different_download_keeps_the_previous_failure_visible() {
        let failed = ("first".to_owned(), "download failed".to_owned());
        let mut error = Some(failed.clone());
        clear_install_error_for(&mut error, "second");
        assert_eq!(error, Some(failed));
        clear_install_error_for(&mut error, "first");
        assert_eq!(error, None);
    }

    #[test]
    fn a_queued_play_waits_for_a_pre_launch_command_to_finish() {
        let mut waiting = queued();
        assert_eq!(take_queued_play(&mut waiting, true), None);
        assert_eq!(
            waiting,
            queued(),
            "a queued Play is kept while another start holds the Stop"
        );
        assert_eq!(take_queued_play(&mut waiting, false), queued());
        assert_eq!(waiting, None, "once no start holds the Stop, it is taken");
    }
}
