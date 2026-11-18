//! The blocky look's Play page and crash notice.
use super::*;

impl Launcher {
    pub(crate) fn play_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        // A Java instance plays Mojang's game: no Horizon version, nothing of Horizon's to prepare.
        let java = self.selected_instance().and_then(|i| i.java.clone());
        let Some(tag) = self
            .selected()
            .cloned()
            .or_else(|| java.is_some().then(String::new))
        else {
            // No hero without a version: the page starts below the top bar like the others.
            let memory = self.memory_row(cx);
            // The first version, downloading since an instance was made to play it; or why it
            // could not be, with a way to try again.
            let failed = match (&self.installing, &self.install_error, &self.releases) {
                (Some(_), _, _) => None,
                (None, Some((tag, e)), _) => Some((
                    tr("Couldn't download Horizon.").to_string(),
                    e.clone(),
                    Some(tag.clone()),
                )),
                (None, None, Load::Failed(e)) => Some((
                    tr("Couldn't get the list of Horizon versions.").to_string(),
                    e.clone(),
                    None,
                )),
                _ => None,
            };
            let waiting = match (&self.installing, &failed) {
                (Some((_, p)), _) => format!("{} {:.0}%", tr("Installing"), p * 100.),
                (None, Some((what, _, _))) => what.clone(),
                (None, None) => tr("No version installed yet.").to_string(),
            };
            let mut actions = div().flex().gap(px(8.));
            if let Some((_, _, tag)) = &failed {
                let tag = tag.clone();
                actions = actions.child(
                    button("retry-first", "Try again", Kind::Primary, &t).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.install_error = None;
                            let release = match (&tag, &this.releases) {
                                (Some(tag), Load::Ready(list)) => {
                                    list.iter().find(|r| &r.tag == tag).cloned()
                                }
                                _ => None,
                            };
                            match release {
                                Some(release) => this.install(&release),
                                None => this.fetch_releases(),
                            }
                            cx.notify();
                        },
                    )),
                );
            }
            actions = actions.child(
                button("go-versions", "Choose a version", Kind::Secondary, &t).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.go(Page::Versions);
                        cx.notify();
                    }),
                ),
            );
            // A failure is a card: what happened as its title, the plain reason, the actions.
            let middle = match &failed {
                Some((what, why, _)) => div().pt(px(24.)).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .max_w(px(560.))
                        .p(px(20.))
                        .rounded(px(16.))
                        .border_1()
                        .border_color(t.danger.opacity(0.45))
                        .bg(t.danger.opacity(0.08))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .child(blocks::icon("info-box", 18., t.danger))
                                .child(text(theme::UI).text_color(t.ink).child(what.clone())),
                        )
                        .child(
                            text(theme::CAPTION)
                                .text_color(t.muted)
                                .child(download_error(why)),
                        )
                        .child(actions),
                ),
                None => centered(&waiting, &t).child(actions),
            };
            return div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .pt(px(28.))
                .child(page_header(
                    "Play",
                    "Start the game, or run a server for others to join.",
                    &t,
                ))
                .child(middle)
                .children(memory);
        };
        let prepared = backend::is_prepared(&tag);
        // Prepared without sounds (arismoko/horizon#1035): Prepare again is the fix, so it stands out.
        let silent = prepared && !backend::has_sounds(&tag);
        let version = java
            .as_ref()
            .map_or_else(|| backend::game_version(&tag), |j| j.minecraft.clone());
        let left = self.download_left();
        let steps = self.preparing.then(|| self.steps(&version, left));

        // Not prepared: an owner can have Horizon download Minecraft from Mojang; without an
        // owner there is nothing to play, so signing in comes first.
        let first = (!prepared && !self.preparing)
            .then_some(self.owner)
            .flatten();
        // No launcher folder with Minecraft on this computer: an owner needs none.
        let no_install = self.settings.minecraft_dir.is_none();
        let resources_value = match first {
            _ if self.preparing => tr("Preparing…").to_string(),
            Some(true) if no_install => trf(
                "Not prepared. Play downloads Minecraft {version} from Mojang for your account and prepares it.",
                &[("version", &version)],
            ),
            Some(true) => trf(
                "Not prepared. Horizon can download Minecraft {version} from Mojang for your account, or use your own install.",
                &[("version", &version)],
            ),
            Some(false) => trf(
                "Not prepared. Sign in with the Microsoft account that owns Minecraft {version} first.",
                &[("version", &version)],
            ),
            None => backend::resources_summary(prepared, !silent, &version),
        };
        let mut prepare = button(
            "prepare",
            if prepared { "Prepare again" } else { "Prepare" },
            if prepared && !silent {
                Kind::Ghost
            } else {
                Kind::Secondary
            },
            &t,
        );
        if self.preparing {
            prepare = disabled(prepare);
        } else {
            prepare = prepare.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.prepare(backend::MinecraftFrom::Install);
                cx.notify();
            }));
        }
        let prepare = match first {
            Some(true) if no_install => div().child(
                button(
                    "prepare-download",
                    "Download and prepare",
                    Kind::Secondary,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.prepare(backend::MinecraftFrom::Mojang);
                    cx.notify();
                })),
            ),
            Some(true) => div()
                .flex()
                .gap(px(8.))
                .child(
                    button("prepare-install", "From my install", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.prepare(backend::MinecraftFrom::Install);
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    button(
                        "prepare-download",
                        "Download and prepare",
                        Kind::Secondary,
                        &t,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.prepare(backend::MinecraftFrom::Mojang);
                        cx.notify();
                    })),
                ),
            Some(false) => div().child(microsoft::sign_in_button("prepare-sign-in", &t).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.sign_in_to_play();
                    cx.notify();
                }),
            )),
            None => div().child(prepare),
        };
        let owner = self.owner == Some(true);
        let minecraft = self.settings.minecraft_dir.as_deref().map_or_else(
            || {
                div()
                    .child(if owner {
                        tr("None needed: Horizon downloads Minecraft from Mojang for your account.")
                    } else {
                        tr("Not found. Choose the folder where your launcher keeps Minecraft.")
                    })
                    .into_any_element()
            },
            |p| {
                div()
                    .font_family(theme::MONO)
                    .child(short(p))
                    .into_any_element()
            },
        );
        let mut rows = div()
            .flex()
            .flex_col()
            // The glass look's sidebar ⋮ menu holds what the Instance row offers.
            .children((!t.glass).then(|| self.instance_row(1, cx)).flatten())
            // The glass look keeps these in the instance sheet (`glass_sheet.rs`).
            .children((!t.glass).then(|| self.export_panel(cx)).flatten())
            .children((!t.glass).then(|| self.launch_panel(cx)).flatten())
            // The glass look keeps Maximum memory in the instance's Edit dialog.
            .children((!t.glass).then(|| self.memory_row(cx)).flatten());
        rows = match &java {
            Some(java) => rows.child(index_row(
                2,
                tr("Game"),
                Some(
                    div()
                        .child(trf(
                            "Minecraft {version} (Java Edition). Play downloads it from Mojang for your account the first time, with the Java it needs.",
                            &[("version", &java.minecraft)],
                        ))
                        .into_any_element(),
                ),
                None,
                &t,
            )),
            // The glass look's Overview ends with the screenshots and the patch log: Prepare is in
            // Play's ⋯ menu, the Minecraft folder in the folder menu, signing in in the top bar.
            None if t.glass => rows,
            None => rows
                .child(index_row(
                    2,
                    tr("Resources"),
                    Some(div().child(resources_value).into_any_element()),
                    Some(prepare.into_any_element()),
                    &t,
                ))
                .child(index_row(
                3,
                tr("Minecraft folder"),
                Some(minecraft),
                Some(
                    button("choose-mc", "Choose…", Kind::Ghost, &t)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_folder(cx)))
                        .into_any_element(),
                ),
                &t,
            )),
        };
        // The top bar shows who plays; a row only when there is something to do about it: no
        // account that owns the game (Play needs one), or offline. The glass look's top bar has
        // Sign in with Microsoft and says Offline itself.
        if !t.glass {
            if self.owner == Some(false) {
                rows = rows.child(index_row(
                    5,
                    tr("Microsoft account"),
                    Some(
                        div()
                            .child(format!("{}.", tr(account::NEEDS_OWNER)))
                            .into_any_element(),
                    ),
                    Some(
                        microsoft::sign_in_button("gate-sign-in", &t)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.sign_in_to_play();
                                cx.notify();
                            }))
                            .into_any_element(),
                    ),
                    &t,
                ));
            } else if self.accounts.selected().is_none() && account::client_id().is_some() {
                rows = rows.child(index_row(
                    5,
                    tr("Playing offline"),
                    Some(
                        div()
                            .child(tr("Online servers need a Microsoft account."))
                            .into_any_element(),
                    ),
                    Some(
                        button("accounts", "Accounts…", Kind::Ghost, &t)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.go(Page::Settings);
                                cx.notify();
                            }))
                            .into_any_element(),
                    ),
                    &t,
                ));
            }
        }

        // The glass look's head (the mockup's `HomeView`) replaces the banner; the preparation's
        // steps stay right under it, where the player looks after pressing Play.
        let banner = if t.glass {
            div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .child(self.glass_home_head(&tag, prepared, cx))
                .children(steps)
                .children(self.glass_build_notice(cx))
        } else {
            let hero = match &java {
                // Its own game: the Minecraft version and the loader, never Horizon's.
                Some(j) => {
                    let game = match &j.loader {
                        Some(l) => format!("{version} · {} {}", l.kind.name(), l.version),
                        None => version.clone(),
                    };
                    self.hero("", &game, true, cx)
                }
                None => self.hero(&tag, &version, prepared, cx),
            };
            let actions = self.home_actions(prepared || java.is_some(), java.is_some(), cx);
            // The banner: the hero and the Play row over the sky (`sky.rs`), or over the player's
            // own picture under a veil of the page colour. It reaches the content's edges and the top
            // bar.
            let veil = |angle: f32, from: f32, to: f32, at: f32| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(gpui::linear_gradient(
                        angle,
                        gpui::linear_color_stop(t.page.opacity(from), 0.),
                        gpui::linear_color_stop(t.page.opacity(to), at),
                    ))
            };
            // At the window's size (`picture_cache`): a chosen picture may be far bigger.
            let side = picture_cache::Look::Fit(self.window_side());
            let picture = self
                .home_background()
                .and_then(|path| self.picture(&path, side, cx))
                .map(|image| {
                    [
                        gpui::img(image)
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .object_fit(gpui::ObjectFit::Cover)
                            .into_any_element(),
                        veil(90., 0.82, 0., 0.62).into_any_element(),
                        veil(180., 0., 0.75, 1.).into_any_element(),
                    ]
                });
            div()
                .relative()
                .flex()
                .flex_col()
                .justify_end()
                .min_h(px(340.))
                .mx(px(-theme::PAGE_GUTTER))
                .px(px(theme::PAGE_GUTTER))
                .pb(px(28.))
                .gap(px(24.))
                .overflow_hidden()
                .children(picture.into_iter().flatten())
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_end()
                        .gap(px(32.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(28.))
                                .child(hero)
                                .child(actions)
                                // The preparation's steps right under Play, where the player looks
                                // after pressing it, not below the fold (arismoko/horizon#666).
                                .children(steps),
                        )
                        // Notes and friends are Horizon's.
                        .children(java.is_none().then(|| self.home_card(&tag, cx))),
                )
        };
        let mut page = div().flex().flex_col().gap(px(28.)).child(banner);
        if let Some(e) = self.prepare_error.clone() {
            page = page.child(notice(e, &t));
        }
        // Home's Stable Update failed, its one retry included: its offer is gone until the list
        // is read again, so Home says why.
        let stable_failed = self.install_error.as_ref().filter(|(tag, _)| {
            self.stable_failed
                .as_ref()
                .is_some_and(|failed| failed.tag == *tag)
        });
        if let Some((_, e)) = stable_failed {
            page = page.child(notice(
                format!("{} {e}", tr("Couldn't download Horizon.")),
                &t,
            ));
        }
        // On the instance, before its tabs: what crashed and the way back into the game.
        if let Some(crash) = self.crash.clone() {
            page = page.child(self.crash_notice(crash, cx));
        }
        page = if t.glass {
            // What Play still needs (and an open export or launch panel), above the tabs.
            page.child(rows).child(self.glass_home_tabs(cx))
        } else {
            page.child(self.home_tabs(rows, cx))
        };
        if let Some(e) = self.client_error.clone() {
            page = page.child(notice(e, &t));
        }
        page
    }

    /// "Horizon crashed": the report and the logs, and where to report it.
    pub(crate) fn crash_notice(&self, crash: Crash, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut actions = div().flex().flex_wrap().gap(px(8.)).mt(px(8.));
        if let Some(report) = crash.report.clone() {
            let open = report.clone();
            actions = actions
                .child(
                    button("crash-open", "Open report", Kind::Secondary, &t)
                        .bg(t.plate)
                        .on_click(move |_, _, cx| crate::glass_toast::open(cx, &open)),
                )
                .child(
                    button(
                        "crash-copy",
                        if crash.copied {
                            "Copied"
                        } else {
                            "Copy report"
                        },
                        Kind::Secondary,
                        &t,
                    )
                    .bg(t.plate)
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if let Ok(text) = std::fs::read_to_string(&report) {
                                copy_to_clipboard(cx, gpui::ClipboardItem::new_string(text));
                                if let Some(c) = &mut this.crash {
                                    c.copied = true;
                                }
                            }
                            cx.notify();
                        },
                    )),
                );
        }
        actions = actions.child(
            button("crash-logs", "Open logs folder", Kind::Secondary, &t)
                .bg(t.plate)
                .on_click(|_, _, cx| {
                    let dir = backend::logs_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    crate::glass_toast::open(cx, &dir);
                }),
        );
        let discord = self.links.discord.clone();
        if !discord.is_empty() {
            actions = actions.child(
                button("crash-discord", "Report on Discord", Kind::Secondary, &t)
                    .bg(t.plate)
                    .on_click(move |_, _, cx| cx.open_url(&discord)),
            );
        }
        actions = actions.child(
            button("crash-dismiss", "Dismiss", Kind::Ghost, &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.crash = None;
                    cx.notify();
                },
            )),
        );
        let help = tr(if crash.report.is_some() {
            "When you report it on Discord, attach the crash report file (or paste it with Copy report) and say what you were doing."
        } else {
            "The game ended without a crash report. When you report it on Discord, attach latest.log from the logs folder and say what you were doing."
        });
        let mut body = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                text(theme::HEADING)
                    .text_color(t.ink)
                    .child(tr("Horizon crashed")),
            );
        if !crash.summary.is_empty() {
            body = body.child(div().text_color(t.ink_soft).child(crash.summary));
        }
        if let Some(report) = &crash.report {
            body = body.child(
                text(theme::MICRO)
                    .text_color(t.muted)
                    .truncate()
                    .child(short(report)),
            );
        }
        body = body.child(actions).child(
            text(theme::CAPTION)
                .mt(px(4.))
                .text_color(t.muted)
                .child(help),
        );
        plate(&t)
            .flex()
            .gap(px(14.))
            .p(px(24.))
            .text_size(px(theme::UI.px))
            .line_height(px(21.))
            .text_color(t.ink)
            .child(div().flex_none().mt(px(11.)).size(px(6.)).bg(t.danger))
            .child(body)
    }
}
