//! The launcher's state set up (`Launcher::new`), its settings saved, page changes, the
//! launcher's own updates and the release list.
use super::*;

impl Launcher {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        horizon_launcher::game_shader_api::draws_here();
        horizon_launcher::storage::eviction::cleanup_after_start();
        let (tx, rx) = channel();
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            this.theme = this.settings_theme(window);
            cx.notify();
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            if this
                .update(cx, |this, cx| this.drain(window_handle, cx))
                .is_err()
            {
                break;
            }
        })
        .detach();
        // Counts the time a partner banner is on screen, until it may be hidden.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_secs(10))
                .await;
            let counting = this.update(cx, |this, cx| {
                let s = &mut this.settings;
                if s.partner_hidden || s.partner_seen_secs >= partner::SEEN_BEFORE_HIDE {
                    return false;
                }
                // On screen on every page: the card in the sidebar, the banner on Servers.
                s.partner_seen_secs += 10;
                if s.partner_seen_secs % 60 == 0 || s.partner_seen_secs >= partner::SEEN_BEFORE_HIDE
                {
                    this.save_settings();
                    cx.notify();
                }
                true
            });
            if !matches!(counting, Ok(true)) {
                break;
            }
        })
        .detach();
        let mut settings = Settings::load();
        i18n::set(settings.language());
        if settings.minecraft_dir.is_none() {
            settings.minecraft_dir = backend::default_minecraft_dir();
        }
        // Release builds on Linux put their entry back in the applications menu when there is
        // none or it starts a program that is gone (a moved or deleted binary), never over one
        // that works; `HORIZON_LAUNCHER_DESKTOP_ENTRY=1` does it in a development build.
        if cfg!(target_os = "linux")
            && settings.desktop_entry()
            && (!cfg!(debug_assertions)
                || std::env::var_os("HORIZON_LAUNCHER_DESKTOP_ENTRY").is_some())
        {
            std::thread::spawn(|| {
                let _ = horizon_launcher::desktop::repair(backend::exe_path());
            });
        }
        let installed = backend::installed();
        if settings
            .selected
            .as_ref()
            .is_none_or(|s| !installed.contains(s))
        {
            settings.selected = installed.first().cloned();
        }
        // Pin the window's chrome (macOS title bar) to the choice before the theme reads
        // `window.appearance()`, so both draw the same look.
        titlebar::apply_appearance(window, settings.appearance);
        let scene = cx.new(|_| scene::Scene::new());
        let sky = cx.new(|cx| sky::SkyView::new(window, cx));
        let rain = cx.new(|_| rain::Rain::new());
        let backdrop =
            cx.new(|_| window_root::Backdrop::new(scene.clone(), sky.clone(), rain.clone()));
        let mut this = Self {
            theme: crate::theme::from_settings(&settings, window.appearance()),
            page: Page::from_env(),
            behind: Page::Play,
            switches: 0,
            content_scroll: Scrollbar::default(),
            nav_strip: nav_strip::NavStrip::default(),
            settings_pane: gpui::ScrollHandle::new(),
            settings_query: line_edit::LineEdit::new(String::new(), 80, 40),
            settings_query_focus: cx.focus_handle(),
            settings_query_focused: false,
            whats_new_open: None,
            settings_marked: 0,
            credit_asked: false,
            version_menu: false,
            credit_picture: None,
            sheet_size: None,
            mods_sticky: sticky::Sticky::default(),
            long_list: cull::Cull::default(),
            opening: (!settings.skip_opening).then(|| {
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                Opening {
                    started: None,
                    reveal: None,
                    focus,
                }
            }),
            updates: Updates::default(),
            checked: None,
            update_error: None,
            // The installer the last Update started could not put the new launcher in place.
            self_update: horizon_launcher::update_handoff::take_failure()
                .map_or(SelfUpdate::Idle, |e| SelfUpdate::Stuck(Some(e))),
            self_update_hidden: false,
            update_install: None,
            instance_install: None,
            install_next: None,
            bar: (0., 0.),
            releases: Load::Loading,
            installed,
            installing: None,
            removing: Vec::new(),
            disk: Vec::new(),
            storage_report: None,
            storage_pending: false,
            storage_confirm: None,
            storage_error: None,
            storage_revoke: None,
            storage_counting: None,
            storage_visit: 0,
            storage_revoke_error: None,
            native_consent_build: None,
            disk_for: Vec::new(),
            install_error: None,
            importing_build: false,
            import_error: None,
            native_step: None,
            native_progress: None,
            first_build: None,
            play_after_build: None,
            native_done: None,
            native_error: None,
            native_failed_mods: Vec::new(),
            native_worlds: None,
            pending_consent: None,
            consent_error: None,
            native_for: None,
            native_built_mods: Vec::new(),
            rooms: Default::default(),
            native_builds: Default::default(),
            native_log: Default::default(),
            confirm_remove: None,
            notes_open: Default::default(),
            notes: Default::default(),
            technical_open: Default::default(),
            picker: glass_home::PickerState::default(),
            accent_strip: false,
            look_sliders: Default::default(),
            hue_area: std::rc::Rc::new(std::cell::Cell::new(None)),
            hue_dragging: false,
            language_menu: false,
            account_menu: false,
            row_menu: Default::default(),
            rail: Default::default(),
            sheet_tab: Default::default(),
            look: horizon_theme::glass::Glass::default_look(),
            menu_closed_at: None,
            wardrobe: None,
            wardrobe_marks: Default::default(),
            wardrobe_reload: false,
            shop: Default::default(),
            shop_status: None,
            shop_kept: None,
            // The first draw takes the launcher theme from Settings.
            restyle: true,
            background_error: None,
            location_target: None,
            location_error: location_page::startup_error(),
            location_note: location_page::startup_note(),
            args_input: line_edit::LineEdit::new(settings.game_args.clone(), 1024, 38),
            args_focused: false,
            args_focus: cx.focus_handle(),
            proxy_input: line_edit::LineEdit::new(
                settings.proxy.clone().unwrap_or_default(),
                256,
                44,
            ),
            proxy_focused: false,
            proxy_focus: cx.focus_handle(),
            settings,
            settings_save_error: None,
            links: backend::Links::default(),
            preparing: false,
            prepare_log: VecDeque::new(),
            log_revision: 0,
            prepare_error: None,
            client: None,
            client_error: None,
            client_started: None,
            client_home: None,
            crash: None,
            servers: servers_page::Servers::new(),
            internet: internet::Internet::new(),
            root_focus: cx.focus_handle(),
            palette: None,
            toast_seen: None,
            accounts: Accounts::load(),
            auth_status: HashMap::new(),
            auth_progress: HashMap::new(),
            profile_heads: profile_heads::Heads::default(),
            auth_checked: None,
            sign_in_target: None,
            auth_busy: HashMap::new(),
            auth_next: 0,
            sign_in: None,
            sign_in_state: None,
            keychain: None,
            owner: None,
            owner_screen_open: false,
            starting_client: None,
            starting_instance: None,
            play_when_prepared: None,
            friend_code: line_edit::LineEdit::new(String::new(), 256, 32),
            friend_focused: false,
            window_width: 0.,
            window_long_side: 0.,
            scale_factor: 1.,
            friend_focus: cx.focus_handle(),
            joining: None,
            quick_play: None,
            client_run: None,
            client_vars: Vec::new(),
            collection: collection::Collection::load(),
            agents: agents_page::AgentsUi::default(),
            playing: None,
            official_players: None,
            tx,
            rx,
            _appearance: appearance,
            _activation: cx.observe_window_activation(window, |_, _, cx| cx.notify()),
            perf: perf::Perf::new("session"),
            install_started: None,
            prepare_started: None,
            download_seen: None,
            home_tab: home_tabs::HomeTab::from_env(),
            home_menu: Default::default(),
            folder_hovered: false,
            list_hover: None,
            list_menu: None,
            home_strip: Default::default(),
            client_since: None,
            instance_mods: Default::default(),
            mod_preview: Default::default(),
            project_icons: Default::default(),
            play_pressed: None,
            play_step: Default::default(),
            bench: perf::Bench::from_env(),
            packs: Vec::new(),
            registry_packs: Vec::new(),
            packs_loaded: None,
            packs_reading: Default::default(),
            pack_open: None,
            pack_filter: None,
            pack_busy: false,
            pack_log: VecDeque::new(),
            pack_error: None,
            found_packs: Vec::new(),
            found_scanning: false,
            starting_pack: None,
            starting_work: None,
            porting: None,
            mods: mods_page::Mods::new(cx),
            github: github_mods_page::Github::new(cx),
            instances: instances_page::Instances::default(),
            worlds: worlds_page::Worlds::default(),
            multiplayer: multiplayer_page::Multiplayer::default(),
            screenshots: screenshots_page::Screenshots::default(),
            shot_blur: Default::default(),
            pictures: Default::default(),
            game_packs: packs_page::Packs::default(),
            shader_answers: 0,
            world_packs: datapacks_page::WorldPacks::default(),
            export: None,
            launch: None,
            memory_slider: Default::default(),
            importing: Default::default(),
            console: Default::default(),
            aside: while_playing::Aside::new(window.window_handle()),
            shortcuts: Default::default(),
            side_servers: horizon_launcher::servers::list(),
            sky,
            scene,
            rain,
            backdrop,
            pushed: None,
            scale: window.scale_factor(),
        };
        if this.location_error.is_some() || this.location_note.is_some() {
            this.page = Page::Settings;
        }
        this.reload_instances();
        if this.bench.is_some() {
            this.perf.keep_local_log(false);
        }
        let tx = this.tx.clone();
        let keychain_tx = this.tx.clone();
        // Asking the keychain can block (D-Bus), so not on this thread.
        let accounts = this.accounts.clone();
        std::thread::spawn(move || {
            let store = account::store();
            let _ = keychain_tx.send(Msg::Keychain(account::no_keychain()));
            let _ = keychain_tx.send(Msg::Owner(accounts.has_owner(store)));
        });
        std::thread::spawn(move || {
            let reused = backend::reuse_prepared_everywhere();
            if !reused.is_empty() {
                let _ = tx.send(Msg::Reused(reused));
            }
        });
        this.load_profile_heads();
        this.load_auth_statuses(true);
        this.fetch_startup_releases();
        this.mods_auto_update();
        this.fetch_links();
        this.watch_shop(cx);
        if home::getting_ready_preview() {
            this.preparing = true;
            this.play_when_prepared = this
                .selected()
                .cloned()
                .zip(this.selected_instance().map(|i| i.id.clone()));
        }
        // `HORIZON_LAUNCHER_PAGE=new-instance`: the dialog open on Modded, for screenshots
        // (`new-instance-horizon`, `new-instance-vanilla`: on those).
        let kind = match std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() {
            Ok("new-instance") => Some(new_instance::NewKind::Modded),
            Ok("new-instance-horizon") => Some(new_instance::NewKind::Horizon),
            Ok("new-instance-vanilla") => Some(new_instance::NewKind::Vanilla),
            _ => None,
        };
        if let Some(kind) = kind {
            this.open_instance_dialog(None);
            this.pick_kind(kind, cx);
        }
        if std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() == Ok("edit-instance") {
            this.sheet_tab = glass_sheet::SheetTab::from_env().unwrap_or_default();
            this.open_instance_dialog(this.selected_instance().cloned());
        }
        if cfg!(debug_assertions)
            && matches!(
                std::env::var("HORIZON_LAUNCHER_PAGE").as_deref(),
                Ok("wardrobe" | "shop")
            )
        {
            this.wardrobe_preview();
        }
        // What the game and the launcher kept, if the player allows it and there is a server.
        this.mod_preview_from_env();
        telemetry::send_in_background();
        telemetry::watch_playing(this.tx.clone());
        telemetry::watch_official(this.tx.clone());
        this
    }

    /// Keeps the current choices in memory if writing fails; every page offers the same retry.
    pub(crate) fn save_settings(&mut self) -> bool {
        self.settings_save_error = self.settings.save().err();
        self.settings_save_error.is_none()
    }

    pub(crate) fn settings_save_notice(&self, cx: &mut Context<Self>) -> Option<Div> {
        let error = self.settings_save_error.as_ref()?;
        let t = self.theme;
        Some(
            div()
                .flex_none()
                .px(px(theme::PAGE_GUTTER))
                .py(px(12.))
                .child(
                    crate::plate::plate(&t.night)
                        .flex()
                        .items_center()
                        .gap(px(16.))
                        .p(px(16.))
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    shell::text(theme::night::UI, t.danger)
                                        .child(tr("Settings were not saved.")),
                                )
                                .child(
                                    shell::text(theme::night::CAPTION, t.ink_soft).child(trf(
                                        "Your changes are still in this window. Retry to save them. {error}",
                                        &[("error", error)],
                                    )),
                                ),
                        )
                        .child(
                            button("settings-save-retry", "Retry", Kind::Secondary, &t)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.save_settings();
                                    cx.notify();
                                })),
                        ),
                ),
        )
    }

    pub(crate) fn go(&mut self, page: Page) {
        // The CLI or another window may have changed the accounts: read them again (a small
        // file, once a switch).
        self.accounts = Accounts::load();
        // Each check reads the keychain (a prompt on macOS): page switches check every account
        // at most once a minute, and an account added meanwhile at once.
        if self
            .auth_checked
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(60))
            || (self.accounts.accounts.iter()).any(|a| !self.auth_status.contains_key(&a.uuid))
        {
            self.load_auth_statuses(false);
        }
        self.load_profile_heads();
        // A Turn off question left open isn't still asked the next time Settings opens.
        self.storage_revoke = None;
        // A Turn off error is read on the visit it shows in: leaving Settings settles it.
        if self.page == Page::Settings && page != Page::Settings {
            self.storage_revoke_error = None;
        }
        self.storage_counting = None;
        self.storage_visit += 1;
        self.wardrobe_reload |= matches!(page, Page::Wardrobe | Page::Shop);
        // The glass look's Settings is a dialog over the page: opening or closing it keeps the
        // page under it as it was (its scroll, no fade).
        let dialog = self.theme.glass
            && (page == Page::Settings || (self.page == Page::Settings && page == self.behind));
        if page != Page::Settings {
            self.behind = page;
        }
        if page != self.page && dialog {
            self.page = page;
        } else if page != self.page {
            self.page = page;
            self.switches += 1;
            self.content_scroll = Scrollbar::default();
            self.long_list = cull::Cull::default();
            self.mods_sticky.reset();
            if page == Page::Mods {
                self.mods.reset_lists();
            }
        }
        self.picker.open = false;
        self.language_menu = false;
        self.mods.browse.sort_menu = false;
        self.account_menu = false;
    }

    pub(crate) fn check_updates(&mut self) {
        self.checked = Some(Instant::now());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Updates(backend::check_updates().map(Box::new)));
        });
    }

    /// Recovery text shared by both sidebar styles and Settings.
    pub(crate) fn launcher_update_help(&self, error: Option<&str>) -> String {
        if let Some(error) = error {
            return trf("the update failed: {error}", &[("error", error)]);
        }
        if backend::package_managed_launcher() {
            return tr("Update this launcher with your package manager.").to_string();
        }
        let folder = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from(tr("Downloads")));
        trf(
            "Close this launcher, move it to {folder}, and open it there to enable updates.",
            &[("folder", &folder.display().to_string())],
        )
    }

    pub(crate) fn launcher_download_available(&self) -> bool {
        !backend::package_managed_launcher()
    }

    pub(crate) fn update_launcher(&mut self, release: Release) {
        if !backend::self_update_allowed()
            || matches!(
                self.self_update,
                SelfUpdate::Downloading(..) | SelfUpdate::Ready(_) | SelfUpdate::Stuck(_)
            )
        {
            return;
        }
        if backend::cannot_replace_itself() {
            self.self_update = SelfUpdate::Stuck(None);
            return;
        }
        let version = backend::launcher_version(&release.tag)
            .unwrap_or_default()
            .to_string();
        self.self_update = SelfUpdate::Downloading(version, 0.);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let progress = |p| {
                let _ = tx.send(Msg::LauncherProgress(p));
            };
            let _ = tx.send(Msg::LauncherUpdated(backend::stage_update(
                &release, progress,
            )));
        });
    }

    /// Update: the window closes, the installer puts the new launcher in place and starts it.
    /// When another launcher already put it there, this one only restarts.
    pub(crate) fn apply_launcher_update(&mut self, cx: &mut App) {
        match horizon_launcher::update_handoff::start_now() {
            Ok(true) => {
                servers_page::keep_running_through_update();
                shut_down(cx)
            }
            Ok(false) => self.self_update = SelfUpdate::Failed(backend::restart()),
            Err(e) => self.self_update = SelfUpdate::Failed(e),
        }
    }

    /// Installs the Horizon version the last check found, keeping the player's choice of
    /// version unless they were on the newest one.
    pub(crate) fn install_update(&mut self) {
        let Some(release) = self.updates.horizon.clone() else {
            return;
        };
        if self.installing.is_some() || self.removing.contains(&release.tag) {
            return;
        }
        self.update_install = Some(release.tag.clone());
        self.install(&release);
    }

    /// The native mods turned on in the instance Play starts, unpacked and checked, with their
    /// bundles: what the warning names and the build compiles.
    pub(crate) fn native_mods_to_build(
        &self,
    ) -> Result<(std::path::PathBuf, Vec<native_mods::Bundle>), String> {
        let folder = native_mods::installed_mods(&instances::selected().home)?;
        let found = native_mods::bundles(&folder)?;
        Ok((folder, found))
    }

    /// What installed native build `version` was made from (`native_status::Build`), read once.
    pub(crate) fn native_build_of(&self, version: &str) -> Option<native_status::Build> {
        if !native_mods::is_native(version) {
            return None;
        }
        self.native_builds
            .borrow_mut()
            .entry(version.to_owned())
            .or_insert_with(|| native_status::Build::read(version))
            .clone()
    }

    pub(crate) fn fetch_releases(&mut self) {
        self.releases = Load::Loading;
        self.notes.clear();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Releases(backend::releases()));
        });
    }

    pub(crate) fn fetch_startup_releases(&mut self) {
        self.checked = Some(Instant::now());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let updates = backend::releases_and_updates(|releases| {
                let _ = tx.send(Msg::Releases(releases));
            });
            let _ = tx.send(Msg::Updates(updates.map(Box::new)));
        });
    }

    pub(crate) fn fetch_links(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Links(backend::links()));
        });
    }
}
