//! The blocky look's top bar, sidebar and instance picker.
use super::*;

impl Launcher {
    /// The top bar: the mark and name over the sidebar (in the title bar on Windows and Linux
    /// client-side decorations), the pages with their icons, then the account and Settings.
    /// The top bar over the pages: their tabs, then the account chip and Settings. No plate: the
    /// sky shows through, under a 1 px rule.
    pub(crate) fn topbar(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let n = t.night;
        let mut nav = div()
            .flex()
            .flex_1()
            .min_w_0()
            .flex_wrap()
            .items_center()
            .gap(px(4.));
        for (page, label, _) in Page::in_top_bar(self.wardrobe_shown()) {
            nav = nav.child(
                shell::tab(format!("nav-{label}"), tr(label), self.page == page, &t).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.go(page);
                        cx.notify();
                    }),
                ),
            );
        }
        // A signed-in account shows its face (its skin, else its UUID's default).
        let (name, how) = self.who_plays();
        let face = self.accounts.selected().map(|a| a.uuid.clone());
        let picture = match face {
            Some(uuid) => (self.profile_heads)
                .draw(Some(&uuid), 28., self.scale, &t)
                .into_any_element(),
            None => div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.))
                .bg(plate::alpha(n.card_hi, n.card_hi_alpha))
                .child(blocks::icon("user", 16., shell::c(n.muted)))
                .into_any_element(),
        };
        let account = shell::chip("account", &t)
            .max_w(px(200.))
            .child(div().relative().child(picture))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        shell::text(theme::night::UI, shell::c(n.ink))
                            .font_weight(FontWeight::SEMIBOLD)
                            .line_height(px(16.))
                            .truncate()
                            .child(name),
                    )
                    .child(
                        shell::text(theme::night::CAPTION, shell::c(n.muted))
                            .truncate()
                            .child(how),
                    ),
            )
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if this.closed_by_this_press(event) {
                    return;
                }
                this.account_menu = !this.account_menu;
                if this.account_menu {
                    // Who can play may have changed elsewhere since the window opened.
                    this.accounts = Accounts::load();
                    this.load_profile_heads();
                }
                cx.notify();
            }));
        let account = div()
            .relative()
            .child(account)
            .children(self.account_menu(cx));
        let settings = shell::icon_button(
            "nav-Settings",
            "sliders",
            40.,
            self.page == Page::Settings,
            &t,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.go(Page::Settings);
            cx.notify();
        }));
        // The last native build's log, when there is one (`native_log_page`).
        let build_log = self.native_log.available.then(|| {
            tab_action("topbar-native-log", "Build log", &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.native_log_open();
                    cx.notify();
                },
            ))
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .min_h(px(theme::TOPBAR))
            .py(px(8.))
            .flex_wrap()
            .min_w_0()
            .pl(px(theme::PAGE_GUTTER - 14.))
            .pr(px(12.))
            .border_b_1()
            .border_color(plate::alpha(n.stroke, n.rule_alpha))
            .child(nav)
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .children(build_log)
                    .child(account)
                    .child(settings),
            )
    }

    /// The sidebar, from the top of the window: the wordmark, Horizon and the imported modpacks,
    /// the player's servers, then the launcher's own update, the community links and its version.
    pub(crate) fn sidebar(&mut self, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let n = t.night;
        self.load_packs();
        let heading = |label: String| shell::heading(label, &t);
        let add = shell::icon_button("add-instance", "plus", 24., false, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.open_instance_dialog(None);
                cx.notify();
            },
        ));
        let items = self.instance_items(heading, cx);
        let mut rows = items.len();
        let list = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(heading(tr("INSTANCES").into()).child(add))
            .children(items);
        let servers: Vec<_> = self
            .servers
            .sidebar_list()
            .map(<[_]>::to_vec)
            .unwrap_or_else(|| self.side_servers.clone());
        let mut list = list;
        if !servers.is_empty() {
            list = list.child(heading(tr("SERVERS").into()));
        }
        rows += servers.len();
        let headings = 1 + usize::from(!servers.is_empty());
        for e in servers {
            let up = self.servers.is_up(&e.dir);
            let open = self.page == Page::Servers && self.servers.open.as_ref() == Some(&e.dir);
            let dir = e.dir.clone();
            list = list.child(
                shell::row(format!("side-server-{}", e.dir.display()), open, &t)
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(36.))
                            .bg(plate::alpha(n.card_hi, n.card_hi_alpha))
                            .child(blocks::icon("server", 18., shell::c(n.ink_soft))),
                    )
                    .child(shell::row_text(
                        e.name.clone(),
                        tr(if up { "Running" } else { "Stopped" }).into(),
                        Some(shell::c(if up { n.success } else { n.muted })),
                        &t,
                    ))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_server(dir.clone());
                        this.go(Page::Servers);
                        cx.notify();
                    })),
            );
        }
        let link = |id: &'static str, label: &'static str, icon: &'static str, url: String| {
            let id = SharedString::from(id);
            track_hover(
                div()
                    .id(id.clone())
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(32.))
                    .px(px(8.))
                    .cursor_pointer()
                    .child(hover_layer(&id, shell::c(n.card_hi).opacity(0.5), 0., 0.))
                    .child(
                        div()
                            .relative()
                            .child(blocks::icon(icon, 16., shell::c(n.muted))),
                    )
                    .child(
                        shell::text(theme::night::UI, shell::c(n.ink_soft))
                            .relative()
                            .child(label),
                    )
                    .on_click(move |_, _, cx| cx.open_url(&url)),
                id,
            )
        };
        let mut foot = div().mt_auto().flex().flex_col().gap(px(2.)).pt(px(12.));
        let update = |line: String| {
            plate::plate(&n)
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(12.))
                .mb(px(6.))
                .child(
                    shell::text(theme::night::UI, shell::c(n.ink))
                        .relative()
                        .child(line),
                )
        };
        match &self.self_update {
            SelfUpdate::Downloading(version, f) => {
                foot = foot.child(
                    update(trf(
                        "Launcher {version}  {percent}%",
                        &[
                            ("version", version),
                            ("percent", &format!("{:.0}", f * 100.)),
                        ],
                    ))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .child(progress_bar(Some((*f, *f)), &t)),
                    ),
                )
            }
            SelfUpdate::Ready(version) => {
                foot = foot.child(
                    update(tr("A new version is available").to_string())
                        .child(
                            shell::text(theme::night::UI, shell::c(n.ink_soft))
                                .relative()
                                .child(trf("Launcher {version}", &[("version", version)])),
                        )
                        .child(div().relative().flex().child(
                            button("update-launcher", "Update", Kind::Primary, &t).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.apply_launcher_update(cx);
                                    cx.notify();
                                }),
                            ),
                        )),
                )
            }
            SelfUpdate::Stuck(error) if !self.self_update_hidden => {
                let why = self.launcher_update_help(error.as_deref());
                let url = backend::download_page(&self.links.website);
                foot = foot.child(
                    update(tr("A new version is available").to_string())
                        .child(
                            shell::text(theme::night::UI, shell::c(n.ink_soft))
                                .relative()
                                .child(why),
                        )
                        .child(
                            div()
                                .relative()
                                .flex()
                                .flex_wrap()
                                .gap(px(6.))
                                .when(self.launcher_download_available(), |actions| {
                                    actions.child(
                                        button(
                                            "download-launcher",
                                            "Download again",
                                            Kind::Primary,
                                            &t,
                                        )
                                        .on_click(move |_, _, cx| cx.open_url(&url)),
                                    )
                                })
                                .child(
                                    button("dismiss-launcher-update", "Dismiss", Kind::Ghost, &t)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.self_update_hidden = true;
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
            }
            _ => {}
        }
        // Sculk's card above the links: seen without scrolling, on every page but Servers, which shows
        // its wide banner. Tall where the sidebar has the height for it besides the wordmark, its rows
        // (52 high each, under headings of 40) and the links, compact otherwise. After a while on
        // screen a "Hide" beside its label puts it away for good.
        if self.page != Page::Servers && !self.settings.partner_hidden {
            let free = f32::from(window.viewport_size().height)
                - theme::TOPBAR
                - 40. * headings as f32
                - 52. * rows as f32
                - 110.;
            let mut side = div().relative().mb(px(8.)).child(partner::card(
                "partner-side",
                theme::SIDEBAR - 20.,
                free < partner::TALL_HEIGHT,
                &t,
            ));
            if self.settings.partner_seen_secs >= partner::SEEN_BEFORE_HIDE {
                let hide = partner::hide_button("partner-hide", &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.settings.partner_hidden = true;
                        this.save_settings();
                        cx.notify();
                    },
                ));
                side = side.child(hide);
            }
            foot = foot.child(side);
        }
        if !self.links.sponsor.is_empty() {
            foot = foot.child(link(
                "sponsor",
                tr("Sponsor"),
                "heart",
                self.links.sponsor.clone(),
            ));
        }
        if !self.links.discord.is_empty() {
            foot = foot.child(link(
                "discord",
                "Discord",
                "message",
                self.links.discord.clone(),
            ));
        }
        if !self.links.roadmap.is_empty() {
            foot = foot.child(link(
                "roadmap",
                tr("Roadmap"),
                "map",
                self.links.roadmap.clone(),
            ));
        }
        if !self.links.docs.is_empty() {
            foot = foot.child(link(
                "docs",
                tr("Documentation"),
                "book",
                self.links.docs.clone(),
            ));
        }
        if !self.links.website.is_empty() {
            foot = foot.child(link(
                "website",
                tr("Website"),
                "globe",
                self.links.website.clone(),
            ));
        }
        foot = foot.child(
            shell::text(theme::night::CAPTION, shell::c(n.muted))
                .id("launcher-version")
                .px(px(8.))
                .pt(px(6.))
                .child(trf("Launcher {version}", &[("version", backend::VERSION)])),
        );
        // Custom title bars carry the mark and name already.
        let brand = (!titlebar::draws_brand(window)).then(|| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(10.))
                .h(px(theme::TOPBAR))
                .px(px(8.))
                .child(mark(24., &t))
                .child(
                    shell::text(theme::night::TITLE, shell::c(n.ink))
                        .font_family(theme::DISPLAY_FONT)
                        .child(tr(identity::WINDOW_TITLE)),
                )
        });
        shell::sidebar_plate(&t)
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(theme::SIDEBAR))
            .h_full()
            .px(px(10.))
            .pb(px(12.))
            .overflow_y_scroll()
            .children(brand)
            .child(list)
            .child(foot)
    }

    /// Whether `event` is the press that just closed a menu from outside it (on the menu's
    /// own button): the button then leaves the menu closed instead of opening it again.
    pub(crate) fn closed_by_this_press(&mut self, event: &ClickEvent) -> bool {
        closed_by_press(self.menu_closed_at.take(), event)
    }

    pub(crate) fn picker(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        // "Nightly · ba6344a", a release's tag: the menu lists the full tags.
        let label = self.selected().map_or_else(
            || tr("No version").into(),
            |tag| {
                horizon_launcher::nightly::label(tag)
                    .or_else(|| build_import::label(tag))
                    .unwrap_or_else(|| tag.clone())
            },
        );
        // As tall as Play: "Version" over the tag, and a chevron.
        let n = t.night;
        let mut wrap = div().relative().child(
            keyboard(div().id("picker"), &t)
                .relative()
                .flex()
                .items_center()
                .gap(px(14.))
                .h(px(theme::night::PLAY))
                .pl(px(14.))
                .pr(px(12.))
                .rounded(px(theme::night::RADIUS_CONTROL))
                .bg(plate::alpha(n.card_hi, n.card_hi_alpha))
                .border_1()
                .border_color(plate::alpha(n.stroke, n.stroke_alpha))
                .cursor_pointer()
                .hover(move |s| s.border_color(plate::alpha(n.stroke, 3. * n.stroke_alpha)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            shell::text(theme::night::CAPTION, t.muted)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(tr("Version").to_string()),
                        )
                        .child(shell::text(theme::night::UI, t.ink).child(label)),
                )
                .child(blocks::icon("chevron-down", 20., t.muted))
                .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                    if !this.closed_by_this_press(event) {
                        this.picker.open = !this.picker.open;
                    }
                    cx.notify();
                })),
        );
        if self.picker.open {
            let mut menu = div()
                .absolute()
                .top(px(theme::night::PLAY + 6.))
                .left_0()
                .w(px(340.))
                .p(px(6.))
                .rounded(px(theme::night::RADIUS_CONTROL))
                .border_1()
                .border_color(plate::alpha(n.stroke, 2. * n.stroke_alpha))
                .bg(t.surface)
                .flex()
                .flex_col()
                .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.picker.open = false;
                    this.menu_closed_at = Some(event.position);
                    cx.notify();
                }));
            // Picking a version keeps the selected instance on it; the launcher's own version
            // (newest unless chosen otherwise on Versions) follows updates.
            let follows = self.launcher_version().cloned();
            for tag in &self.installed {
                let selected = self.selected() == Some(tag);
                let tag = tag.clone();
                let latest = follows.as_ref() == Some(&tag);
                menu = menu.child(
                    div()
                        .id(SharedString::from(format!("pick-{tag}")))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .h(px(32.))
                        .px(px(10.))
                        .rounded(px(theme::RADIUS))
                        .font_family(theme::MONO)
                        .text_size(px(theme::CAPTION.px))
                        .text_color(t.ink)
                        .cursor_pointer()
                        .hover(move |s| s.bg(t.plate_soft))
                        .child(div().min_w_0().truncate().child(tag.clone()))
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap(px(8.))
                                .when(latest, |d| {
                                    d.child(caps(tr("auto-update")).flex_none().text_color(t.muted))
                                })
                                .when(selected, |d| d.child(div().size(px(6.)).bg(t.accent))),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.pin_version((!latest).then(|| tag.clone()));
                            this.picker.open = false;
                            cx.notify();
                        })),
                );
            }
            menu = menu.child(div().my(px(6.)).h(px(1.)).bg(t.rule)).child(
                div()
                    .id("manage")
                    .flex()
                    .items_center()
                    .h(px(32.))
                    .px(px(10.))
                    .rounded(px(theme::RADIUS))
                    .text_size(px(theme::LABEL.px))
                    .text_color(t.muted)
                    .cursor_pointer()
                    .hover(move |s| s.bg(t.plate_soft).text_color(t.ink))
                    .child(tr("Manage versions"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.go(Page::Versions);
                        this.picker.open = false;
                        cx.notify();
                    })),
            );
            wrap = wrap.child(gpui::deferred(menu).with_priority(1));
        }
        wrap
    }
}
