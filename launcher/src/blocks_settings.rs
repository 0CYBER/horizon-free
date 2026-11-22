//! The blocky look's Settings page, the native build consent and the settings rows.
use super::*;

impl Launcher {
    pub(crate) fn settings_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let toggle = switch("auto-install", self.settings.auto_install(), &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.settings.install_new_versions = Some(!this.settings.auto_install());
                this.save_settings();
                if this.settings.auto_install() {
                    this.install_update();
                }
                cx.notify();
            }),
        );
        let prune = switch("auto-prune", self.settings.auto_prune(), &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.prune_old_versions = Some(!this.settings.auto_prune());
                this.save_settings();
                cx.notify();
            },
        ));
        let opening = switch("opening", !self.settings.skip_opening, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.skip_opening = !this.settings.skip_opening;
                this.save_settings();
                cx.notify();
            },
        ));
        // Linux: the launcher's entry in the applications menu (`horizon_launcher::desktop`).
        let menu_entry = switch("desktop-entry", self.settings.desktop_entry(), &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                let on = !this.settings.desktop_entry();
                this.settings.desktop_entry = Some(on);
                this.save_settings();
                let _ = if on {
                    horizon_launcher::desktop::install(backend::exe_path()).map(|_| ())
                } else {
                    horizon_launcher::desktop::remove()
                };
                cx.notify();
            }),
        );
        let horizon = match (&self.updates.horizon, &self.update_error) {
            (Some(r), _) => trf("{version} available", &[("version", update_name(r))]),
            (None, Some(e)) => e.clone(),
            (None, None) if self.checked.is_some() => tr("Up to date").to_string(),
            _ => tr("Checking…").to_string(),
        };
        let horizon_action = self
            .updates
            .horizon
            .as_ref()
            .filter(|_| self.installing.is_none())
            .map(|_| {
                button("settings-install", "Install", Kind::Secondary, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.install_update();
                        cx.notify();
                    }))
                    .into_any_element()
            });
        let launcher = match &self.self_update {
            SelfUpdate::Downloading(v, p) => {
                let done = format!("{:.0}", p * 100.);
                let said = trf(
                    "downloading {version}, {percent}%",
                    &[("version", v), ("percent", &done)],
                );
                format!("{}  ·  {said}", backend::VERSION)
            }
            SelfUpdate::Ready(v) => format!(
                "{}  ·  {}",
                backend::VERSION,
                if horizon_launcher::update_handoff::installs_on_close() {
                    trf(
                        "{version} installs when you close Horizon",
                        &[("version", v)],
                    )
                } else {
                    trf("{version} available", &[("version", v)])
                }
            ),
            SelfUpdate::Stuck(Some(e)) => format!(
                "{}  ·  {}",
                backend::VERSION,
                trf("the update failed: {error}", &[("error", e)])
            ),
            SelfUpdate::Stuck(None) => format!(
                "{}  ·  {}",
                backend::VERSION,
                self.launcher_update_help(None)
            ),
            SelfUpdate::Failed(e) => format!(
                "{}  ·  {}",
                backend::VERSION,
                trf("the update failed: {error}", &[("error", e)])
            ),
            SelfUpdate::Idle => match &self.updates.launcher {
                Some(r) => format!(
                    "{}  ·  {}",
                    backend::VERSION,
                    trf(
                        "{version} available",
                        &[(
                            "version",
                            backend::launcher_version(&r.tag).unwrap_or_default()
                        )]
                    )
                ),
                None if self.checked.is_some() && self.update_error.is_none() => {
                    format!("{}  ·  {}", backend::VERSION, tr("up to date"))
                }
                None => backend::VERSION.to_string(),
            },
        };
        let launcher_action = match &self.self_update {
            SelfUpdate::Ready(_) => Some(
                button("update-settings", "Update now", Kind::Secondary, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.apply_launcher_update(cx);
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
            SelfUpdate::Stuck(_) if self.launcher_download_available() => {
                let url = backend::download_page(&self.links.website);
                Some(
                    button("download-settings", "Download again", Kind::Secondary, &t)
                        .on_click(move |_, _, cx| cx.open_url(&url))
                        .into_any_element(),
                )
            }
            SelfUpdate::Downloading(..) | SelfUpdate::Stuck(_) => None,
            // A newer launcher is known: the button gets it, rather than checking again.
            SelfUpdate::Idle if self.updates.launcher.is_some() => {
                let release = self.updates.launcher.clone();
                let version = release
                    .as_ref()
                    .and_then(|r| backend::launcher_version(&r.tag))
                    .unwrap_or_default();
                let label = trf("Update to {version}", &[("version", version)]);
                let get = button("update-launcher", label, Kind::Secondary, &t);
                Some(if backend::self_update_allowed() {
                    get.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(release) = release.clone() {
                            this.update_launcher(release);
                        }
                        cx.notify();
                    }))
                    .into_any_element()
                } else {
                    // This copy doesn't replace itself (a development build, or updates
                    // switched off): the download page has the new one.
                    let url = backend::download_page(&self.links.website);
                    get.on_click(move |_, _, cx| cx.open_url(&url))
                        .into_any_element()
                })
            }
            _ => Some(
                // The glass look's Updates tile has the mockup's filled button.
                button(
                    "check",
                    "Check now",
                    if t.glass {
                        Kind::Secondary
                    } else {
                        Kind::Ghost
                    },
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.check_updates();
                    cx.notify();
                }))
                .into_any_element(),
            ),
        };
        let mono = |s: String| div().font_family(theme::MONO).child(s).into_any_element();
        let plain = |s: &str| div().child(tr(s).to_string()).into_any_element();
        if t.glass {
            return self.glass_settings(
                glass_settings::Built {
                    install: toggle,
                    prune,
                    opening,
                    menu_entry,
                    horizon,
                    horizon_action,
                    launcher,
                    launcher_action,
                },
                cx,
            );
        }
        let (accounts, n) = self.account_rows(cx);
        let group = |label: &'static str, icon: &'static str| {
            section_title(label, icon, &t).mt(px(20.)).mb(px(12.))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .child(page_header(
                "Settings",
                "Accounts, updates, and the launcher itself.",
                &t,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(group("Account", "user").mt_0())
                    .child(accounts)
                    .child(group("Updates", "reload"))
                    .child(index_row(n + 1, "Horizon", Some(plain(&horizon)), horizon_action, &t))
                    .child(index_row(
                        n + 1,
                        tr("Launcher"),
                        Some(plain(&launcher)),
                        launcher_action,
                        &t,
                    ))
                    .child(index_row(
                        n + 1,
                        tr("Install new versions automatically"),
                        Some(plain(
                            "Your selected version only changes if you were playing the newest one.",
                        )),
                        Some(toggle.into_any_element()),
                        &t,
                    ))
                    .child(index_row(
                        n + 2,
                        tr("Delete old versions automatically"),
                        Some(
                            div()
                                .child(trf(
                                    "Keeps the {count} newest, the one in use and the ones your instances use. Deleted versions install again from the Versions page.",
                                    &[("count", &self.settings.keep_newest().to_string())],
                                ))
                                .into_any_element(),
                        ),
                        Some(prune.into_any_element()),
                        &t,
                    ))
                    .child(group("Launcher", "monitor"))
                    .child(self.launcher_theme_row(cx))
                    .child(self.accent_row(cx))
                    .child(self.home_background_row(cx))
                    .child(self.language_row(cx))
                    .child(self.while_playing_row(n + 3, cx))
                    .child(index_row(
                        n + 3,
                        tr("Show the Horizon logo at startup"),
                        Some(plain("Shown for a moment while the version list loads.")),
                        Some(opening.into_any_element()),
                        &t,
                    ))
                    .child(group("Network", "globe"))
                    .child(self.proxy_row(n + 4, cx))
                    .child(group("Game", "gamepad"))
                    .child(self.game_architecture_row(n + 5, cx))
                    .child(self.game_args_row(n + 6, cx))
                    .child(group("AI", "terminal"))
                    .child(self.no_ai_row(n + 6, cx))
                    .child(group("About", "info-box"))
                    .child(self.location_row(n + 9, cx))
                    .child(index_row(
                        n + 10,
                        tr("Logs"),
                        Some(mono(format!(
                            "{}  ·  {}",
                            short(&backend::logs_dir()),
                            tr("crash reports beside it")
                        ))),
                        Some(
                            button("open-logs", "Open", Kind::Secondary, &t)
                                .on_click(|_, _, cx| {
                                    let dir = backend::logs_dir();
                                    let _ = std::fs::create_dir_all(&dir);
                                    crate::glass_toast::open(cx, &dir);
                                })
                                .into_any_element(),
                        ),
                        &t,
                    ))
                    .when(cfg!(target_os = "linux"), |d| {
                        d.child(index_row(
                            n + 11,
                            tr("Applications menu"),
                            Some(plain(if self.settings.desktop_entry() {
                                "Horizon and its icon are in your desktop's menu."
                            } else {
                                "Not in your desktop's menu."
                            })),
                            Some(menu_entry.into_any_element()),
                            &t,
                        ))
                    }),
            )
            .child(self.collection_section(cx))
            .children(self.agents_section(cx))
    }

    pub(crate) fn request_native_build_consent(&mut self) {
        self.native_consent_build = None;
        self.pending_consent = Some(PendingConsent::Build(
            native_status::enabled(&self.mods.home())
                .into_iter()
                .map(|(id, _, _)| id)
                .collect(),
        ));
    }

    /// Cancel and Escape share the transition; neither can execute a pending action.
    pub(crate) fn cancel_native_consent(&mut self) {
        self.play_after_build = None;
        self.native_consent_build = None;
        if let Err(error) = native_consent::resolve(
            &mut self.pending_consent,
            false,
            true,
            &mut self.settings,
            &self.mods.home(),
        ) {
            self.mods.message = Some(Err(error));
        }
        self.consent_error = None;
        self.instance_mods_changed();
    }

    pub(crate) fn native_consent(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        // The mods asked about, when known (a build with none enabled names none).
        let name = self
            .pending_consent
            .as_ref()
            .map(PendingConsent::name)
            .filter(|name| !name.is_empty());
        let build = matches!(self.pending_consent, Some(PendingConsent::Build(_)));
        let bytes = horizon_launcher::rust_toolchain::host().map(|host| host.download_bytes());
        let size = bytes
            .map(storage::format_bytes)
            .unwrap_or_else(|| tr("Unavailable on this platform").into());
        div().absolute().inset_0().occlude().flex().items_center().justify_center().bg(crate::glass_ui::scrim())
            .child(plate(&t).w(px(520.)).p(px(28.)).bg(t.surface).rounded(px(16.)).border_1().border_color(t.rule).shadow_lg().flex().flex_col().gap(px(18.))
                .child(div().size(px(48.)).rounded(px(12.)).bg(t.warn.opacity(0.12)).flex().items_center().justify_center().child(crate::glass_ui::icon("information-circle", 26., t.warn)))
                .child(div().text_size(px(24.)).font_weight(FontWeight::BOLD).child(tr("Allow native mods?")))
                .children(name.map(|name| div().font_weight(FontWeight::BOLD).child(name)))
                .child(tr("These mods are programs that run inside your game. Like any program you install, they can read and change your files. Allow them only if you trust who made them."))
                .children(horizon_launcher::rust_toolchain::installed().is_none().then(|| trf("Horizon will first download a compiler ({size}) to build them.", &[("size", &size)])))
                .child(tr("You can take this permission back in Settings > Storage."))
                .children(self.consent_error.clone().map(|error| div().text_color(t.danger).child(error)))
                .children(bytes.is_none().then(|| div().text_color(t.danger).child(tr("Native mods can't be built on this computer, so they can't be allowed here."))))
                .child(div().flex().justify_end().gap(px(12.))
                    .child(button("native-cancel", "Cancel", Kind::Secondary, &t).w(px(120.)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.cancel_native_consent();
                        cx.notify();
                    })))
                    .child({
                        let allow = button("native-allow", if build { tr("Allow and build") } else { tr("Allow") }, Kind::Primary, &t).w(px(160.));
                        if bytes.is_none() {
                            disabled(allow)
                        } else {
                            allow.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        match native_consent::resolve(&mut this.pending_consent, true, true, &mut this.settings, &this.mods.home()) {
                            Err(error) => this.consent_error = Some(error),
                            Ok(Some(action)) => { this.consent_error = None; this.storage_revoke_error = None; match action {
                                PendingConsent::Build(_) => {
                                    let (base, back_up) = this.native_consent_build.take().unwrap_or_default();
                                    this.build_native_on(base, back_up)
                                }
                                PendingConsent::Toggle(entry) => this.mods_switch(&entry),
                                PendingConsent::Install(request) => this.mods_install(request),
                                PendingConsent::Github(found, only) => this.github_install(found, only),
                                PendingConsent::Local(id) => {
                                    this.mods.message = horizon_launcher::browser::java::set_enabled(&this.mods.home(), &id, true).err().map(Err);
                                    this.instance_mods_changed();
                                }
                            } }
                            Ok(None) => {},
                        }
                        cx.notify();
                    }))
                        }
                    })
                ))
    }

    /// Settings' Game arguments row: the extra arguments the game starts with, kept in
    /// `settings.json` and appended to the launcher's own (quotes group a word with spaces).
    pub(crate) fn game_architecture_row(&self, n: usize, cx: &mut Context<Self>) -> Div {
        if !backend::can_choose_game_architecture() {
            return div();
        }
        let t = self.theme;
        let picker = segments(
            "game-architecture",
            &[
                (backend::GameArchitecture::Auto, "Automatic"),
                (backend::GameArchitecture::X64, "x64"),
                (backend::GameArchitecture::Arm64, "ARM64"),
            ],
            self.settings.game_architecture,
            &t,
            {
                let this = cx.entity().downgrade();
                move |choice, _, cx| {
                    let _ = this.update(cx, |this, cx| {
                        if this.settings.game_architecture == choice {
                            return;
                        }
                        this.settings.game_architecture = choice;
                        this.save_settings();
                        // The release and update lists contain the selected game's assets.
                        this.releases = Load::Loading;
                        this.updates = backend::Updates::default();
                        this.fetch_startup_releases();
                        cx.notify();
                    });
                }
            },
        );
        index_row(
            n,
            tr("Game architecture"),
            Some(
                div()
                    .child(tr(
                        "Automatic follows this launcher's architecture. On Windows ARM, choose x64 or ARM64. Releases without ARM64 use the signed x64 build.",
                    ))
                    .into_any_element(),
            ),
            Some(picker.into_any_element()),
            &t,
        )
    }

    pub(crate) fn game_args_row(&self, n: usize, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let focused = self.args_focused;
        let field = div()
            .id("game-args")
            .track_focus(&self.args_focus)
            .flex()
            .items_center()
            .w(px(300.))
            .h(px(30.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .cursor_text()
            .child(line_edit::content(
                &self.args_input,
                &t,
                focused,
                false,
                false,
                tr("For example --server-thread"),
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.args_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.args_input, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Escape => window.blur(cx),
                    line_edit::Key::Edited => {
                        this.settings.game_args = this.args_input.text().to_string();
                        this.save_settings();
                    }
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.args_input);
        if glass_ui::flat() {
            return glass_ui::setting_row_stacked(
                tr("Game arguments"),
                Some(
                    div()
                        .child(tr("Extra arguments for the game, passed through as-is (quotes group a word with spaces). Applies the next time the game starts."))
                        .into_any_element(),
                ),
                glass_ui::wide_field(field, focused).into_any_element(),
            );
        }
        index_row(
            n,
            tr("Game arguments"),
            Some(
                div()
                    .child(tr("Extra arguments for the game, passed through as-is (quotes group a word with spaces). Applies the next time the game starts."))
                    .into_any_element(),
            ),
            Some(field.into_any_element()),
            &t,
        )
    }

    /// Settings' Proxy row: the proxy every download and sign-in goes through, kept in
    /// `settings.json` and read back by `proxy.rs` on each request.
    pub(crate) fn proxy_row(&self, n: usize, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let focused = self.proxy_focused;
        let field = div()
            .id("proxy")
            .track_focus(&self.proxy_focus)
            .flex()
            .items_center()
            .w(px(300.))
            .h(px(30.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .cursor_text()
            .child(line_edit::content(
                &self.proxy_input,
                &t,
                focused,
                false,
                false,
                tr("For example http://127.0.0.1:7890"),
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.proxy_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.proxy_input, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Escape => window.blur(cx),
                    line_edit::Key::Edited => this.save_proxy(),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.proxy_input);
        // A half-typed address is no reason to complain; one left behind is.
        let detail = match (
            focused,
            horizon_launcher::proxy::refuse(self.proxy_input.text()),
        ) {
            (false, Some(why)) => div().text_color(t.danger).child(why).into_any_element(),
            _ => div()
                .child(tr(
                    "Downloads, sign-ins and updates go through this proxy (http://host:port, \
                     socks5://host:port). Empty means none: HTTPS_PROXY is used instead when the \
                     launcher was started with one.",
                ))
                .into_any_element(),
        };
        if glass_ui::flat() {
            return glass_ui::setting_row_stacked(
                tr("Proxy"),
                Some(detail),
                glass_ui::wide_field(field, focused).into_any_element(),
            );
        }
        index_row(
            n,
            tr("Proxy"),
            Some(detail),
            Some(field.into_any_element()),
            &t,
        )
    }

    /// Keeps the Proxy field in `settings.json`, and tells the parts that build their own agents
    /// (`horizon-port-pack`) which proxy to use.
    pub(crate) fn save_proxy(&mut self) {
        let value = self.proxy_input.text().trim().to_owned();
        self.settings.proxy = (!value.is_empty()).then_some(value);
        self.save_settings();
        horizon_launcher::proxy::announce(self.settings.proxy.clone());
    }

    /// Settings' Microsoft accounts: who is signed in and plays, the sign-in with its code. The
    /// rows are numbered from 01; returns them and how many there are.
    pub(crate) fn account_rows(&mut self, cx: &mut Context<Self>) -> (Div, usize) {
        let t = self.theme;
        let (mut rows, mut n) = self.microsoft_rows(cx);
        n += 1;
        let (title, detail, action) = self.sign_in_parts(cx);
        rows = rows.child(index_row(
            n,
            title,
            Some(detail.into_any_element()),
            action,
            &t,
        ));
        (rows, n)
    }

    /// The Microsoft accounts signed in here: each one's head, name and state, Play as,
    /// Refresh and Sign out.
    pub(crate) fn microsoft_rows(&mut self, cx: &mut Context<Self>) -> (Div, usize) {
        let t = self.theme;
        let mut rows = div().flex().flex_col();
        let mut n = 0;
        let selected = self.accounts.selected.clone();
        for (i, a) in self.accounts.accounts.clone().into_iter().enumerate() {
            let plays = selected.as_ref() == Some(&a.uuid);
            let uuid = a.uuid.clone();
            let mut actions = div().flex().gap(px(4.));
            if plays {
                actions = actions.child(
                    button(format!("offline-{i}"), "Play offline", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            let mut accounts = Accounts::load();
                            accounts.selected = None;
                            if accounts.save().is_ok() {
                                this.accounts = accounts;
                            }
                            cx.notify();
                        }),
                    ),
                );
            } else {
                let uuid = uuid.clone();
                actions = actions.child(
                    button(format!("select-{i}"), "Play as", Kind::Secondary, &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.play_as(&uuid);
                            cx.notify();
                        }),
                    ),
                );
            }
            let sign_in_progress = self
                .sign_in_target
                .as_ref()
                .filter(|a| a.uuid == uuid)
                .and_then(|_| match self.sign_in_state.as_ref() {
                    Some(SignInState::Starting) => Some("Asking Microsoft…"),
                    Some(SignInState::Code(_)) => Some("Waiting for browser sign-in…"),
                    Some(SignInState::Working(phase)) => Some(phase.label()),
                    _ => None,
                });
            let progress = sign_in_progress
                .or_else(|| self.auth_progress.get(&uuid).map(|phase| phase.label()));
            let busy = self.auth_busy.contains_key(&uuid) || sign_in_progress.is_some();
            let status = self.auth_status.get(&uuid);
            let needs_sign_in =
                status.is_some_and(|s| s.state == account::session::State::SignInRequired);
            let refresh_id = uuid.clone();
            let refresh = button(
                format!("refresh-{i}"),
                if busy {
                    "Checking…"
                } else if needs_sign_in {
                    "Sign in again"
                } else {
                    "Refresh"
                },
                Kind::Ghost,
                &t,
            );
            actions = actions.child(if busy {
                disabled(refresh)
            } else {
                refresh.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if needs_sign_in {
                        this.re_sign_in(&refresh_id);
                    } else {
                        this.refresh_account(refresh_id.clone());
                    }
                    cx.notify();
                }))
            });
            let sign_out = button(format!("sign-out-{i}"), "Sign out", Kind::Ghost, &t);
            actions = actions.child(if busy {
                disabled(sign_out)
            } else {
                sign_out.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let Some(generation) = this.begin_auth_job(&uuid) else {
                        return;
                    };
                    let (tx, uuid) = (this.tx.clone(), uuid.clone());
                    std::thread::spawn(move || {
                        let mut accounts = Accounts::load();
                        let _ = tx.send(Msg::AccountProgress(
                            uuid.clone(),
                            generation,
                            account::session::Phase::SigningOut,
                        ));
                        let result = accounts
                            .remove(account::store(), &uuid)
                            .and_then(|()| accounts.save())
                            .map(|()| accounts.has_owner(account::store()));
                        let _ = tx.send(Msg::AccountRemoved(uuid, generation, result));
                    });
                    cx.notify();
                }))
            });
            n += 1;
            rows = rows.child(account_row(
                Some(self.profile_heads.draw(Some(&a.uuid), 32., self.scale, &t)),
                a.name.clone(),
                Some(
                    div()
                        .child(tr(if plays {
                            "Microsoft account · plays"
                        } else {
                            "Microsoft account"
                        }))
                        .child(account_status_line(status, progress, &t))
                        .when(!busy, |row| {
                            row.when_some(status.filter(|s| s.message != s.label()), |row, s| {
                                row.child(div().child(tr(&s.message).to_string()))
                            })
                        })
                        .into_any_element(),
                ),
                actions,
                &t,
            ));
        }
        (rows, n)
    }

    /// Adding a Microsoft account, or signing one in again: what the row is called, what is
    /// happening (the code to enter, a problem), and its buttons.
    pub(crate) fn sign_in_parts(
        &self,
        cx: &mut Context<Self>,
    ) -> (String, Div, Option<AnyElement>) {
        let t = self.theme;
        let configured = account::client_id().is_some();
        let busy = matches!(
            self.sign_in_state,
            Some(SignInState::Starting | SignInState::Code(_) | SignInState::Working(_))
        );
        let detail = match &self.sign_in_state {
            _ if !configured => div().child(tr(account::NOT_CONFIGURED)),
            Some(SignInState::Starting) => div().child(tr("Asking Microsoft…")),
            Some(SignInState::Working(phase)) => div().child(tr(phase.label())),
            Some(SignInState::Code(code)) => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(trf(
                    "Open {link} in your browser and enter this code:",
                    &[("link", &code.verification_uri)],
                ))
                .child(
                    text(theme::HEADING)
                        .font_family(theme::MONO)
                        .text_color(t.ink)
                        .child(code.user_code.clone()),
                )
                .when(!microsoft::has_private_window(), |col| {
                    col.child(tr("Open this link in a private window to pick another account."))
                })
                .child({
                    let uri = code.verification_uri.clone();
                    div().flex().child(
                        button(
                            "sign-in-other",
                            "Use a different Microsoft account",
                            Kind::Ghost,
                            &t,
                        )
                        .on_click(move |_, _, cx| microsoft::use_different_account(&uri, cx)),
                    )
                }),
            Some(SignInState::Done(Ok(a))) => {
                div().child(trf("Signed in as {name}.", &[("name", &a.name)]))
            }
            Some(SignInState::Done(Err(e))) => div().text_color(t.danger).child(e.clone()),
            None if self.accounts.accounts.is_empty() => div().child(tr(
                "Horizon needs a Microsoft account that owns Minecraft: Java Edition, like other launchers. It also plays online servers with your name and skin.",
            )),
            None => div().child(tr("Add another account.")),
        };
        let detail = if let Some(target) = &self.sign_in_target {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(trf(
                    "Use the Microsoft account for {name}.",
                    &[("name", &target.name)],
                ))
                .child(detail)
        } else {
            detail
        };
        let detail = match self.keychain.clone() {
            Some(said) if !self.accounts.accounts.is_empty() => div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(detail)
                .child(div().child(said)),
            _ => detail,
        };
        let action = if !configured {
            None
        } else if let Some(SignInState::Code(code)) = &self.sign_in_state {
            let (uri, user_code) = (code.verification_uri.clone(), code.user_code.clone());
            let (private_uri, link_uri) = (uri.clone(), uri.clone());
            // A private window picks the account; without one the link is for the player to carry over.
            let private = if microsoft::has_private_window() {
                button(
                    "sign-in-private",
                    "Open in private window",
                    Kind::Secondary,
                    &t,
                )
                .on_click(move |_, _, cx| microsoft::open_private(&private_uri, cx))
            } else {
                button("sign-in-copy-link", "Copy link", Kind::Secondary, &t).on_click(
                    move |_, _, cx| {
                        copy_to_clipboard(cx, gpui::ClipboardItem::new_string(link_uri.clone()))
                    },
                )
            };
            Some(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(
                        button("sign-in-copy", "Copy code", Kind::Secondary, &t).on_click(
                            move |_, _, cx| {
                                copy_to_clipboard(
                                    cx,
                                    gpui::ClipboardItem::new_string(user_code.clone()),
                                )
                            },
                        ),
                    )
                    .child(private)
                    .child(
                        button("sign-in-open", "Open link", Kind::Secondary, &t)
                            .on_click(move |_, _, cx| cx.open_url(&uri)),
                    )
                    .child(
                        button("sign-in-cancel", "Cancel", Kind::Ghost, &t).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.cancel_sign_in();
                                cx.notify();
                            },
                        )),
                    )
                    .into_any_element(),
            )
        } else if busy {
            Some(
                button("sign-in-cancel-working", "Cancel", Kind::Ghost, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.cancel_sign_in();
                        cx.notify();
                    }))
                    .into_any_element(),
            )
        } else {
            let target = self.sign_in_target.as_ref().map(|a| a.uuid.clone());
            let targeted = target.is_some();
            // Re-signing one account names it; adding one is Microsoft's own button.
            let sign_in = if targeted {
                button("sign-in", "Sign in again", Kind::Secondary, &t)
            } else {
                microsoft::sign_in_button("sign-in", &t)
            };
            let mut actions = div().flex().gap(px(4.)).child(sign_in.on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    if let Some(uuid) = &target {
                        this.re_sign_in(uuid);
                    } else {
                        this.start_sign_in();
                    }
                    cx.notify();
                },
            )));
            if targeted {
                actions = actions.child(
                    button("sign-in-cancel-target", "Cancel", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.cancel_sign_in();
                            cx.notify();
                        }),
                    ),
                );
            }
            Some(actions.into_any_element())
        };
        let title = match &self.sign_in_target {
            Some(target) => trf("Sign in again as {name}", &[("name", &target.name)]),
            None if self.accounts.accounts.is_empty() => tr("Microsoft account").to_owned(),
            None => tr("Another account").to_owned(),
        };
        (title, detail, action)
    }
}
