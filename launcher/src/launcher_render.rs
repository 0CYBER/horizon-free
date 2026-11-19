//! The launcher's frame (`Render for Launcher`) and its opening.
use super::*;

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        if specimen::wanted() {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .bg(t.side)
                .children(titlebar::render(&t, window))
                .child(specimen::render(&t));
        }
        if std::mem::take(&mut self.restyle) {
            self.theme = self.settings_theme(window);
        }
        glass_ui::set_on(self.theme.glass);
        glass_ui::set_light(!self.theme.dark);
        glass::set_light(!self.theme.dark);
        self.look = self.theme.look;
        glass_ui::set_look(self.look);
        glass_ui::set_effects(self.settings.effects);
        glass::set_look(self.look);
        // The mockup tints the scene with any accent but its violet.
        let tint = (self.theme.dark && self.look.accent != horizon_theme::glass::ACCENT)
            .then(|| horizon_theme::glass::mix_oklab(self.look.accent, 0x767676, 0.58));
        let blur = self.look.blur;
        let performance = self.settings.performance();
        // Performance mode's plates are solid: no glass, no blur, nothing sampled from the scene.
        // So are an OLED theme's, over its own black sky.
        let sunset = self.theme.glass && self.theme.dark && self.theme.scene;
        glass::set_plain(
            performance || (self.theme.dark && !self.theme.scene),
            self.theme.edge,
        );
        // A frame per wheel notch's step is what Performance mode saves: notches jump there.
        glide::set_on(!performance);
        // Reduced motion (the player's or the system's) holds GPUI's own animations too.
        sky::set_preference(self.settings.reduce_motion);
        cx.set_reduce_motion(motion::still());
        let blur = if performance { 0. } else { blur };
        // Hypr Neon's night rains, except in performance mode.
        let rains = self.theme.dark && self.theme.edge.is_some() && !performance;
        let accent = self.look.accent;
        self.scale = window.scale_factor();
        self.perf.frame();
        if self.bench.as_ref().is_some_and(perf::Bench::scrolling) {
            self.bench_scroll(window);
        }
        self.servers.note_focus(window, cx);
        self.friend_focused = self.friend_focus.is_focused(window);
        self.mods.note_focus(window);
        self.github.note_focus(window);
        self.args_focused = self.args_focus.is_focused(window);
        self.proxy_focused = self.proxy_focus.is_focused(window);
        self.settings_query_focused = self.settings_query_focus.is_focused(window);
        let size = window.viewport_size();
        self.window_width = f32::from(size.width);
        self.window_long_side = f32::from(size.width).max(f32::from(size.height));
        self.scale_factor = window.scale_factor();
        let shown = if t.glass && self.page == Page::Settings {
            self.behind
        } else {
            self.page
        };
        let content = match shown {
            Page::Play => self.play_page(cx),
            Page::Versions => self.versions_page(cx),
            Page::Mods => self.mods_page(cx),
            Page::Servers => self.servers_page(cx),
            Page::Wardrobe => self.wardrobe_page(cx),
            Page::Shop => self.shop_page(cx),
            Page::Settings => self.settings_page(cx),
        };
        self.pictures.free_retired(window);
        self.wardrobe_marks.free_replaced(window);
        self.shop.free_replaced(window);
        // The new page fades in and rises a few pixels, as the website's tabs do (`hzl-pop`).
        let content = div()
            .relative()
            .w_full()
            // Let the page shrink inside the padded flex row so long notices can wrap.
            .min_w_0()
            .child(content)
            .with_animation(
                ElementId::named_usize("page", self.switches),
                motion::eased(motion::PAGE),
                |d, p| d.opacity(p).top(px(motion::PAGE_RISE * (1. - p))),
            );
        let (reveal, held) = self.opening_phase(window);
        let ui = ease_out_cubic((reveal - 0.3) / 0.7);
        if self.opening.is_none() && window.focused(cx).is_none() {
            window.focus(&self.root_focus, cx);
        }
        // The glass look draws the new design's scene in the dark appearance; the light look and
        // the blocky looks keep the baked sky. The sky is tiles that don't depend on the
        // window's size, so a resize costs no rebake.
        glass::begin_build(window);
        let painted = (!sunset).then(|| {
            let (night, size) = (t.night, (size.width.into(), size.height.into()));
            // A new blur, size or theme is blurred on a worker; the plates keep the last one
            // until it is ready, so dragging Blur never holds a frame.
            if let Some(job) = sky::frost_later(&night, size) {
                cx.spawn(async move |this, cx| {
                    let done = cx.background_spawn(async move { job() }).await;
                    glass::frost_done(done);
                    let _ = this.update(cx, |_, cx| cx.notify());
                })
                .detach();
            }
            (night, size, window.scale_factor())
        });
        // What is behind the launcher, handed to its views only when it changed: reading them
        // on every frame would make each sky frame draw the launcher again too.
        let keep = frame::keep_clear(
            size.width.into(),
            size.height.into(),
            self.page == Page::Play,
        );
        let pushed = Pushed {
            keep,
            performance,
            tint,
            blur,
            rains,
            accent,
            painted,
            back: window_root::Back {
                side: t.side,
                sunset,
                rains,
                // The sunset is Home's scenery; elsewhere it is dimmed behind the work.
                dim: sunset && self.page != Page::Play,
            },
        };
        if self.pushed != Some(pushed) {
            self.pushed = Some(pushed);
            self.scene.update(cx, |scene, cx| {
                scene.set_performance(performance, cx);
                scene.set_look(tint, blur, cx);
            });
            self.rain
                .update(cx, |rain, cx| rain.set(rains, accent, keep, cx));
            if let Some((night, size, scale)) = painted {
                self.sky
                    .update(cx, |sky, cx| sky.set(night, size, scale, keep, cx));
            }
            self.backdrop.update(cx, |b, cx| b.set(pushed.back, cx));
        }
        // Dark: the glass frame over the scene; Light keeps the night one over the dawn.
        let (topbar, sidebar) = if t.glass {
            (
                self.glass_topbar(window, cx),
                self.glass_rail(window, cx)
                    .ml(px(frame::MARGIN))
                    .mb(px(frame::MARGIN))
                    .into_any_element(),
            )
        } else {
            (self.topbar(cx), self.sidebar(window, cx).into_any_element())
        };
        let row_menu = if t.glass {
            self.row_menu(window, cx)
        } else {
            None
        };
        let page = self
            .content_scroll
            .wrap(
                div()
                    .id("content")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    // A scrolled page is drawn from its layer, not
                    // rendered: the Mods page's card asks, once the
                    // offset has moved, whether it must follow.
                    .on_scroll(cx.listener(|this, _: &gpui::ScrollEvent, window, cx| {
                        if this.page != Page::Mods {
                            return;
                        }
                        cx.on_next_frame(window, |this, _, cx| {
                            if this.mods_sticky.moved(&this.content_scroll.handle) {
                                cx.notify();
                            }
                        });
                    }))
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .min_w_0()
                            .px(px(theme::PAGE_GUTTER))
                            // The glass look's page heads hold their
                            // own 40 px above the title.
                            .pt(px(if self.page == Page::Play || t.glass {
                                0.
                            } else {
                                28.
                            }))
                            .pb(px(48.))
                            .child(content),
                    ),
                &t,
            )
            .flex_1()
            .min_h_0()
            .when(t.glass, |d| d.mt(px(crate::glass::CONTENT_INSET)));
        let mut root = div()
            .id("root")
            .track_focus(&self.root_focus)
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" && this.pending_consent.is_some() {
                    this.cancel_native_consent();
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                match k.key.as_str() {
                    "tab" => {
                        KEYBOARD.store(true, Ordering::Relaxed);
                        if k.modifiers.shift {
                            window.focus_prev(cx);
                        } else {
                            window.focus_next(cx);
                        }
                    }
                    "escape" if this.welcome_escape() => {}
                    "escape" if this.consent_escape() => {}
                    "escape" if this.native_log_escape() => {}
                    "escape" if this.servers_escape() => {}
                    "escape" if this.instance_escape() => {}
                    "escape" if this.settings_escape() => {}
                    "escape" if this.screenshot_escape() => {}
                    "left" if this.screenshot_step(false) => {}
                    "right" if this.screenshot_step(true) => {}
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            // ⌘K (Ctrl+K elsewhere) wherever the focus is, even in a field: the palette.
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                let command = if cfg!(target_os = "macos") {
                    k.modifiers.platform
                } else {
                    k.modifiers.control
                };
                if this.theme.glass && command && !k.modifiers.shift && k.key == "k" {
                    this.palette_toggle(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .capture_any_mouse_down(|_, _, _| KEYBOARD.store(false, Ordering::Relaxed))
            .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, window, cx| {
                let size = window.viewport_size();
                this.scene
                    .update(cx, |scene, cx| scene.point(ev.position, size, cx));
                this.rain.update(cx, |rain, cx| rain.poke(cx));
            }))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .font_family(theme::FONT)
            .text_color(t.ink)
            .overflow_hidden()
            // First, so it takes a wheel notch before any scroll area does.
            .child(glide::wheel())
            .child(if t.glass {
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .size_full()
                    .opacity(ui)
                    .top(px(theme::RISE * (1. - ui)))
                    .child(topbar)
                    // The page keeps the collapsed rail's room; the rail opens over it, so the
                    // page never moves or squeezes when the pointer reaches for an instance.
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_1()
                            .min_h_0()
                            .child(div().flex_none().w(px(frame::MARGIN + frame::RAIL)))
                            .child(div().flex().flex_col().flex_1().min_w_0().child(page))
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left_0()
                                    .flex()
                                    .child(sidebar),
                            ),
                    )
            } else {
                div()
                    .relative()
                    .flex()
                    .size_full()
                    .opacity(ui)
                    .top(px(theme::RISE * (1. - ui)))
                    .child(sidebar)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(topbar)
                            .child(page),
                    )
            });
        root = root.children(row_menu);
        if self.page == Page::Servers {
            if let Some(dialog) = self.create_dialog(cx) {
                root = root.child(dialog);
            }
        }
        root = self.import_drop_target(root, cx);
        root = root.children(self.import_note(cx));
        if let Some(dialog) = self.instance_dialog(cx) {
            root = root.child(dialog);
        }
        if t.glass && self.page == Page::Settings {
            root = root.child(self.settings_dialog(cx));
        }
        if let Some(panel) = self.native_log_panel(cx) {
            root = root.child(panel);
        }
        if self.page == Page::Play {
            if let Some(viewer) = self.screenshot_viewer(cx) {
                root = root.child(viewer);
            }
        }
        if t.glass {
            root = root.children(self.toast_float());
        }
        if let Some(palette) = self.palette_overlay(window, cx) {
            root = root.child(palette);
        }
        if self.pending_consent.is_some() {
            root = root.child(self.native_consent(cx));
        }
        if self.welcoming() {
            root = root.child(self.welcome_screen(cx));
        } else if self.asking_consent() {
            root = root.child(self.consent_screen(cx));
        }
        if self.asking_owner() {
            root = root.child(self.owner_screen(cx));
        }
        if let Some(opening) = &self.opening {
            // The wordmark arrives letter by letter, then gives way to the launcher; a click or
            // a key skips ahead.
            let gone = 1. - ease_out_cubic(reveal / 0.6);
            let word = div()
                .flex()
                .font_family(theme::PIXEL)
                .text_size(px(theme::WORDMARK.px * 0.5))
                .line_height(px(theme::WORDMARK.px * 0.5))
                .font_weight(FontWeight::BOLD)
                .text_color(t.ink)
                .children("HORIZON".chars().enumerate().map(|(i, c)| {
                    let at = held - i as f32 * theme::STAGGER.as_secs_f32() * 1.5;
                    let p = ease_out_cubic(at / theme::ENTER.as_secs_f32());
                    div()
                        .relative()
                        .opacity(p)
                        .top(px(theme::RISE * 2. * (1. - p)))
                        .child(c.to_string())
                }));
            root = root.child(
                div()
                    .id("opening")
                    .track_focus(&opening.focus)
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(t.page.opacity(gone))
                    .when(reveal == 0., |d| d.cursor_pointer())
                    .child(
                        div()
                            .relative()
                            .top(px(-24. * (1. - gone)))
                            .opacity(gone)
                            .child(word),
                    )
                    .when(reveal == 0., |d| {
                        d.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.skip_opening();
                            cx.notify();
                        }))
                        .on_key_down(cx.listener(
                            |this, _: &KeyDownEvent, _, cx| {
                                this.skip_opening();
                                cx.notify();
                            },
                        ))
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .relative()
            .size_full()
            .children(titlebar::render(&t, window))
            .children(self.settings_save_notice(cx))
            .child(root.flex_1().min_h_0())
            .children(titlebar::resize_handles(window))
    }
}

/// What the launcher's frame hands the views behind it (the scene, the rain, the painted sky).
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Pushed {
    keep: rain::Keep,
    performance: bool,
    tint: Option<u32>,
    blur: f32,
    rains: bool,
    accent: u32,
    painted: Option<(horizon_theme::night::Night, (f32, f32), f32)>,
    back: window_root::Back,
}

impl Launcher {
    /// Where the opening is: the reveal's progress (1 when there is no opening) and how long
    /// the wordmark has been showing, in seconds.
    pub(crate) fn opening_phase(&mut self, window: &mut Window) -> (f32, f32) {
        // Reduced motion: the wordmark's arrival and the reveal are all motion, so no opening.
        if motion::still() {
            self.opening = None;
        }
        let loaded = !matches!(self.releases, Load::Loading);
        let Some(opening) = &mut self.opening else {
            return (1., 10.);
        };
        let held = opening.started.get_or_insert_with(Instant::now).elapsed();
        if opening.reveal.is_none() && ((held >= OPENING_HOLD && loaded) || held >= OPENING_MAX) {
            opening.reveal = Some(Instant::now());
        }
        let reveal = opening.reveal.map_or(0., |at| {
            at.elapsed().as_secs_f32() / OPENING_REVEAL.as_secs_f32()
        });
        if reveal >= 1. {
            self.opening = None;
            return (1., 10.);
        }
        window.request_animation_frame();
        (reveal, held.as_secs_f32())
    }

    /// `horizon-launcher benchmark`: scroll the page a little on every frame, back to the top at
    /// the bottom, so the frame times measure scrolling.
    pub(crate) fn bench_scroll(&mut self, window: &mut Window) {
        let handle = &self.content_scroll.handle;
        let mut offset = handle.offset();
        offset.y -= px(24.);
        if -offset.y > handle.max_offset().y {
            offset.y = px(0.);
        }
        handle.set_offset(offset);
        window.request_animation_frame();
    }

    pub(crate) fn skip_opening(&mut self) {
        if let Some(opening) = &mut self.opening {
            opening.reveal.get_or_insert_with(Instant::now);
        }
    }
}
