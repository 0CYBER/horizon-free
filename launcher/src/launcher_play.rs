//! Installing and removing builds, preparing and starting the game, and the accounts' sign-in.
use super::*;

impl Launcher {
    /// After a crash with no report: when the game's output ends while its Vulkan renderer
    /// started, a native crash in the graphics driver (arismoko/horizon#746), switches the game to
    /// `backend::vulkan_crash_fallback` (DirectX 12 on Windows, OpenGL on Linux; only where the
    /// player left the graphics API on Auto). The API's label when it did.
    pub(crate) fn switched_from_vulkan(&self) -> Option<&'static str> {
        let (api, label) = backend::vulkan_crash_fallback(std::env::consts::OS)?;
        if !backend::vulkan_start_crashed(&self.prepare_log) {
            return None;
        }
        // The GPU is the same for every profile: the game home's setting and the crashed run's.
        let home = backend::game_home();
        let mut homes = vec![home.clone()];
        homes.extend(self.client_home.clone().filter(|h| *h != home));
        let mut switched = false;
        for home in homes {
            match backend::switch_from_auto(&home, api) {
                Ok(changed) => switched |= changed,
                Err(e) => eprintln!("Horizon launcher: could not switch to {label}: {e}"),
            }
        }
        switched.then_some(label)
    }

    /// The version Play starts: the selected instance's own, else the launcher's.
    pub(crate) fn selected(&self) -> Option<&String> {
        self.selected_instance()
            .and_then(|i| self.version_of(i))
            .or_else(|| self.launcher_version())
    }

    /// The launcher's version: what instances without one of their own play.
    pub(crate) fn launcher_version(&self) -> Option<&String> {
        self.settings
            .selected
            .as_ref()
            .filter(|s| self.installed.contains(s))
    }

    /// Installs `release` for an instance made or edited to play it, after the install running
    /// now if there is one.
    pub(crate) fn install_for_instance(&mut self, release: Release) {
        match &self.installing {
            Some((now, _)) if *now == release.tag => {}
            Some(_) => self.install_next = Some(release),
            None => {
                self.instance_install = Some(release.tag.clone());
                self.install(&release);
            }
        }
    }

    pub(crate) fn install(&mut self, release: &Release) {
        if release.asset.is_none() || self.removing.contains(&release.tag) {
            return;
        }
        let (release, tx) = (release.clone(), self.tx.clone());
        let tag = release.tag.clone();
        self.installing = Some((tag.clone(), 0.0));
        self.install_started = Some(Instant::now());
        self.bar = (0., 0.);
        self.install_error = None;
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
        self.settings.selected =
            selection_after_removal(&self.installed, self.settings.selected.take());
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
        if !self.can_import_build() {
            return;
        }
        self.importing_build = true;
        self.import_error = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Imported(build_import::import(&path, None)));
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

    /// [`Self::prepare`] for `tag` rather than the selected version.
    pub(crate) fn prepare_version(&mut self, tag: String, from: backend::MinecraftFrom) {
        self.prepare_tag = Some(tag.clone());
        horizon_launcher::mojang::CANCEL.store(false, Ordering::Relaxed);
        let minecraft = self
            .settings
            .minecraft_dir
            .clone()
            .unwrap_or_else(|| horizon_platform::prepare::artifacts::minecraft_dir(None));
        self.prepare_log.clear();
        self.prepare_error = None;
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
    }

    pub(crate) fn play(&mut self) {
        let java = self
            .selected_instance()
            .filter(|i| i.java.is_some())
            .cloned();
        if let Some(instance) = java {
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
        let Some(tag) = self.selected().cloned() else {
            return;
        };
        if !self.can_start(&tag) {
            return;
        }
        self.starting_pack = None;
        self.start_client(tag, self.selected_instance().map(|i| i.id.clone()));
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
        starts_now(backend::is_prepared(tag), self.preparing, running)
    }

    /// Whether the selected instance's version can start now, from a shortcut (a server's Join,
    /// a world's Play, a friend's code): Horizon's only, never a Java instance.
    pub(crate) fn can_start_selected(&self) -> bool {
        self.selected_instance().is_none_or(|i| i.java.is_none())
            && self.selected().is_some_and(|tag| self.can_start(tag))
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
