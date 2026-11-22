//! Quick play, joining a friend, and the blocky look's Versions page.
use super::*;

impl Launcher {
    /// Join a friend: a code field and Join, which starts the game straight
    /// into the friend's world (errors show in the game).
    /// Plays the selected instance straight into a world (`--world`) or a server (`--server`):
    /// flags every Horizon release takes.
    pub(crate) fn quick_play(&mut self, flag: &'static str, value: String) {
        self.quick_play = Some((flag, value));
        self.play();
        if self.starting_client.is_none() {
            self.quick_play = None;
        }
    }

    pub(crate) fn join_friend(&mut self) {
        if horizon_friends::invite::parse(self.friend_code.text()).is_none() {
            return;
        }
        self.joining = Some(self.friend_code.text().to_string());
        self.play();
        if self.starting_client.is_none() {
            self.joining = None;
        }
    }

    /// Use and Remove for an installed version; Remove asks once more before it deletes.
    pub(crate) fn version_actions(&self, tag: &str, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let confirming = self.confirm_remove.as_deref() == Some(tag);
        let (t1, t2) = (tag.to_string(), tag.to_string());
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .children(
                self.use_version_action(tag)
                    .filter(|_| !confirming)
                    .map(|(id, tip)| {
                        button(format!("use-{tag}"), "Use", Kind::Ghost, &t)
                            .tooltip(crate::glass_ui::tip(tip))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                if let Err(e) = this.use_version_for(&id, &t1) {
                                    this.install_error = Some((t1.clone(), e));
                                }
                                cx.notify();
                            }))
                    }),
            )
            .child(
                button(
                    format!("remove-{tag}"),
                    if confirming {
                        "Remove for good"
                    } else {
                        "Remove"
                    },
                    if confirming {
                        Kind::Secondary
                    } else {
                        Kind::Ghost
                    },
                    &t,
                )
                .when(confirming, |b| b.text_color(t.danger))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if this.confirm_remove.as_ref() == Some(&t2) {
                        this.remove(&t2);
                    } else {
                        this.confirm_remove = Some(t2.clone());
                    }
                    cx.notify();
                })),
            )
    }

    /// One installed build made here on the Versions page, a native build or an imported one:
    /// what it was made from, and Use and Remove.
    pub(crate) fn local_build_row(
        &self,
        tag: &str,
        on_disk: &dyn Fn(&String) -> Option<u64>,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let selected = self.selected().map(String::as_str) == Some(tag);
        let mut meta = Vec::new();
        if let Some(about) = native_mods::about(tag) {
            meta.push(trf("Built on {base}", &[("base", &about.base)]));
        }
        if let Some(about) = build_import::about(tag) {
            let base = horizon_launcher::nightly::label(&about.base).unwrap_or(about.base);
            meta.push(trf("Game files from {base}", &[("base", &base)]));
            meta.push(about.file);
        }
        if let Some(n) = on_disk(&tag.to_string()) {
            meta.push(trf("{size} on disk", &[("size", &megabytes(n))]));
        }
        let right = if self.removing.iter().any(|r| r == tag) {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .w(px(180.))
                .child(progress_bar(None, &t))
                .child(
                    text(theme::MICRO)
                        .text_color(t.muted)
                        .child(tr("Removing…")),
                )
        } else {
            self.version_actions(tag, cx)
        };
        let error = self
            .install_error
            .as_ref()
            .filter(|(e, _)| e == tag)
            .map(|(_, e)| e.clone());
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .py(px(14.))
            .px(px(16.))
            .bg(t.panel)
            .border_2()
            .border_color(t.outline)
            .children(blocks::edges(t.bevel, t.lip, theme::BEVEL))
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(4.))
                            .child(version_tag(tag).text_color(t.ink))
                            .child(caps(meta.join("  ·  ")).text_color(t.muted)),
                    )
                    .children(selected.then(|| caps(tr("In use")).text_color(t.accent)))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .w(px(190.))
                            .flex_none()
                            .child(right),
                    ),
            )
            .children(error.map(|e| notice(e, &t)))
    }

    /// The installed builds of one kind made here (`is`: native or imported) under their caption,
    /// with a notice above them; None when there are none.
    pub(crate) fn local_builds(
        &self,
        title: &str,
        is: fn(&str) -> bool,
        notice_: Option<&str>,
        on_disk: &dyn Fn(&String) -> Option<u64>,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let t = self.theme;
        let tags: Vec<&String> = self
            .installed
            .iter()
            .chain(&self.removing)
            .filter(|v| is(v))
            .collect();
        (!tags.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(caps(tr(title)).text_color(t.muted))
                .children(notice_.map(|n| notice(n.to_string(), &t)))
                .children(
                    tags.into_iter()
                        .map(|tag| self.local_build_row(tag, on_disk, cx)),
                )
        })
    }

    pub(crate) fn versions_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let header = page_header(
            "Versions",
            "Builds from GitHub. Each one installs into its own folder.",
            &t,
        );
        // Measured again when the installed versions change.
        if self.disk_for != self.installed {
            self.disk_for = self.installed.clone();
            let (tags, tx) = (self.installed.clone(), self.tx.clone());
            std::thread::spawn(move || {
                let disk = tags.into_iter().map(|t| {
                    let n = backend::disk_use(&t);
                    (t, n)
                });
                let _ = tx.send(Msg::DiskUse(disk.collect()));
            });
        }
        let on_disk = |tag: &String| {
            let found = self.disk.iter().find(|(t, _)| t == tag);
            found
                .filter(|_| self.installed.contains(tag))
                .map(|(_, n)| *n)
        };
        let used: u64 = self.installed.iter().filter_map(on_disk).sum();
        if t.glass {
            return self.glass_versions(&on_disk, used, cx);
        }
        // The list's own action at the end of the line above it: a build the player has.
        let import: AnyElement = if self.importing_build {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .w(px(180.))
                .child(progress_bar(None, &t))
                .child(
                    text(theme::MICRO)
                        .text_color(t.muted)
                        .child(tr("Importing…")),
                )
                .into_any_element()
        } else {
            button("import-build", "Import a game build…", Kind::Secondary, &t)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.import_build(cx)))
                .into_any_element()
        };
        let sponsor = (!self.links.sponsor.is_empty()).then(|| {
            plate(&t)
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .p(px(24.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            text(theme::HEADING)
                                .text_color(t.ink)
                                .child(tr("Support the work below")),
                        )
                        .child(text(theme::UI).text_color(t.muted).child(tr(
                            "Horizon is built in the open. Sponsors fund the next release.",
                        ))),
                )
                .child(
                    button("sponsor", "Sponsor on GitHub", Kind::Secondary, &t)
                        .bg(t.plate)
                        .on_click({
                            let url = self.links.sponsor.clone();
                            move |_: &ClickEvent, _, cx| cx.open_url(&url)
                        }),
                )
        });
        let page = div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .child(header)
            .children(sponsor)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .child(text(theme::UI).text_color(t.muted).when(used > 0, |d| {
                        d.child(trf(
                            "Installed versions use {size}.",
                            &[("size", &megabytes(used))],
                        ))
                    }))
                    .child(import),
            )
            .children(self.import_error.clone().map(|e| notice(e, &t)));
        // Builds made here, not downloaded, are listed apart from the releases.
        let page = page
            .children(self.local_builds(
                "Native mod builds",
                native_mods::is_native,
                None,
                &on_disk,
                cx,
            ))
            .children(self.local_builds(
                "Imported builds",
                build_import::is_imported,
                Some(tr(
                    "Unsigned: you picked these programs yourself, and Horizon can't \
                    check where they came from. Updates never replace them.",
                )),
                &on_disk,
                cx,
            ));
        let releases = match &self.releases {
            Load::Loading => return page.child(centered(tr("Loading releases…"), &t)),
            Load::Failed(e) => {
                return page.child(
                    centered(&trf("Couldn't load releases. {error}", &[("error", e)]), &t).child(
                        button("retry", "Try again", Kind::Secondary, &t).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.fetch_releases();
                                cx.notify();
                            },
                        )),
                    ),
                );
            }
            Load::Ready(r) if r.is_empty() => {
                return page.child(centered(tr("No releases yet."), &t));
            }
            Load::Ready(r) => r,
        };
        // A hundred releases: only the rows near the screen are drawn.
        let keep = |i: usize| {
            let tag = &releases[i].tag;
            self.installing.as_ref().is_some_and(|(t, _)| t == tag)
                || self.install_error.as_ref().is_some_and(|(t, _)| t == tag)
                || self.confirm_remove.as_ref() == Some(tag)
                || self.removing.contains(tag)
        };
        let list = self.long_list.clone().column(
            &self.content_scroll.handle,
            releases.len(),
            8.,
            keep,
            |i| {
                let release = releases[i].clone();
                let notes = self.notes_section(&release, cx);
                let tag = release.tag.clone();
                let installed = self.installed.contains(&tag);
                let selected = self.selected() == Some(&tag);
                let mut meta = vec![release.date.clone()];
                if let Some((_, size)) = release.asset {
                    meta.push(megabytes(size));
                }
                if let Some(n) = on_disk(&tag) {
                    meta.push(trf("{size} on disk", &[("size", &megabytes(n))]));
                }
                let summary = release
                    .name
                    .split_once(": ")
                    .map(|(_, rest)| rest.to_string())
                    .unwrap_or_else(|| release.name.clone());
                let right: gpui::AnyElement = if let Some((_, p)) =
                    self.installing.as_ref().filter(|(t, _)| *t == tag)
                {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .w(px(180.))
                        .child(progress_bar(Some(self.bar), &t))
                        .child(
                            text(theme::MICRO)
                                .w(px(36.))
                                .text_color(t.muted)
                                .child(format!("{:.0}%", p * 100.)),
                        )
                        .into_any_element()
                } else if self.removing.contains(&tag) {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .w(px(180.))
                        .child(progress_bar(None, &t))
                        .child(
                            text(theme::MICRO)
                                .text_color(t.muted)
                                .child(tr("Removing…")),
                        )
                        .into_any_element()
                } else if installed && backend::needs_game_architecture_install(&tag, &release) {
                    let mut b = button(
                        format!("install-selected-architecture-{tag}"),
                        tr("Install selected build"),
                        Kind::Secondary,
                        &t,
                    );
                    if self.installing.is_some() {
                        b = disabled(b);
                    } else {
                        b = b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.install(&release);
                            cx.notify();
                        }));
                    }
                    b.into_any_element()
                } else if installed {
                    self.version_actions(&tag, cx).into_any_element()
                } else if release.asset.is_none() {
                    let (line, download) = horizon_launcher::refusals::no_build_card();
                    let line = text(theme::CAPTION).text_color(t.muted).child(line);
                    match download {
                        Some(url) => line
                            .id(SharedString::from(format!("no-build-{tag}")))
                            .cursor_pointer()
                            .on_click(move |_: &ClickEvent, _, cx| cx.open_url(&url))
                            .into_any_element(),
                        None => line.into_any_element(),
                    }
                } else {
                    let mut b = button(format!("install-{tag}"), "Install", Kind::Secondary, &t);
                    if self.installing.is_some() {
                        b = disabled(b);
                    } else {
                        b = b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.install(&release);
                            cx.notify();
                        }));
                    }
                    b.into_any_element()
                };
                let status = if selected {
                    Some(caps(tr("In use")).text_color(t.accent))
                } else if installed {
                    Some(caps(tr("Installed")).text_color(t.ink_soft))
                } else {
                    None
                };
                let error = self
                    .install_error
                    .as_ref()
                    .filter(|(t, _)| *t == tag)
                    .map(|(_, e)| e.clone());
                let id = SharedString::from(format!("release-{tag}"));
                let row = div()
                    .id(id.clone())
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .py(px(14.))
                    .px(px(16.))
                    .bg(t.panel)
                    .border_2()
                    .border_color(t.outline)
                    .children(blocks::edges(t.bevel, t.lip, theme::BEVEL))
                    .child(hover_layer(&id, t.raised, 0., 0.))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(4.))
                                    .child(version_tag(&tag).text_color(t.ink))
                                    .child(
                                        text(theme::UI)
                                            .text_color(t.muted)
                                            .truncate()
                                            .child(summary),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_end()
                                    .gap(px(2.))
                                    .flex_none()
                                    .children(status)
                                    .child(caps(meta.join("  ·  ")).text_color(t.muted)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .w(px(190.))
                                    .flex_none()
                                    .child(right),
                            ),
                    )
                    .children(error.map(|e| notice(e, &t)))
                    .child(notes);
                track_hover(row, id)
            },
        );
        page.child(list)
    }
}
