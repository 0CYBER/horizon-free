//! The options pages' built layout: every widget of a page, the list's top
//! and bottom and its content height, made once per size and revision
//! (`OptionsUi::layout`) and read by the drawing, the hit tests and the scroll.
use super::*;

/// A page's built layout: its widgets, the list's top and bottom and its
/// content height. `OptionsUi` makes it once per size and revision; every
/// reader of the page's rectangles reads this.
#[derive(Clone, Debug, PartialEq)]
pub struct OptionsLayout {
    widgets: Vec<Widget>,
    top: f32,
    bottom: f32,
    content: f32,
}

impl OptionsLayout {
    /// The layout's parts: widgets, list top, list bottom, content height.
    pub(super) fn parts(&self) -> (&[Widget], f32, f32, f32) {
        (&self.widgets, self.top, self.bottom, self.content)
    }
}
impl OptionsUi {
    /// Something the page layout reads changed: make it again at the next
    /// read, once.
    pub(super) fn relayout(&mut self) {
        self.discard_draw();
        self.rev = self.rev.wrapping_add(1);
    }
    /// The game changed state the page layout reads (the mod browser's
    /// listings arriving, say): the next read makes the layout once.
    pub fn layout_changed(&mut self) {
        self.relayout();
    }
    /// The Agent page's status lines and recording flag, from the game's
    /// poll: the page lays out again only when one of them changed.
    pub fn set_agent_status(&mut self, status: Vec<String>, recording: bool) {
        if self.agent_status != status || self.agent_recording != recording {
            self.agent_status = status;
            self.agent_recording = recording;
            self.relayout();
        }
    }
    /// The page's layout at `(w, h)`: made on the first read after an open, a
    /// resize or a revision change, then read as is.
    pub(super) fn built_layout(
        &mut self,
        s: &Settings,
        w: f32,
        h: f32,
    ) -> std::sync::Arc<OptionsLayout> {
        // The scroll is an input too: a drag, the wheel or a focus move can
        // change it without a revision bump, and the rows move with it.
        let key = (w, h, self.rev, self.scroll.to_bits());
        if let Some((cached, layout)) = &self.layout_cache
            && *cached == key
        {
            return std::sync::Arc::clone(layout);
        }
        let (widgets, top, bottom, content) = self.layout(s, w, h);
        // `layout` clamps the scroll; key the settled value.
        let key = (w, h, self.rev, self.scroll.to_bits());
        let layout = std::sync::Arc::new(OptionsLayout {
            widgets,
            top,
            bottom,
            content,
        });
        self.layout_cache = Some((key, std::sync::Arc::clone(&layout)));
        #[cfg(test)]
        {
            self.layouts_built += 1;
        }
        layout
    }
    /// The layout built straight from the current state, for the caching
    /// test: the cached one must equal this after every operation.
    #[cfg(test)]
    pub(super) fn layout_now(&mut self, s: &Settings, w: f32, h: f32) -> OptionsLayout {
        let (widgets, top, bottom, content) = self.layout(s, w, h);
        OptionsLayout {
            widgets,
            top,
            bottom,
            content,
        }
    }
    /// Build every widget of the current page.
    pub(super) fn layout(&mut self, s: &Settings, w: f32, h: f32) -> (Vec<Widget>, f32, f32, f32) {
        let mut out = Vec::new();
        let mut rows = ListRows::default();
        let page = self.page();
        let cx = (w / 2.0).floor();
        if matches!(
            page,
            Page::Video | Page::Controls | Page::KeyBinds | Page::ModBrowser
        ) {
            self.settings_search.x = cx - 100.0;
            self.settings_search.y = 17.0;
        }
        let done = |y: f32| button(cx - 100.0, y, 200.0, tr("gui.done"), Act::Done);
        // (list top, list bottom, content height)
        let (mut top, mut bottom, mut content) = (33.0, h - 33.0, 0.0);
        match page {
            Page::Main => {
                out.push(Widget {
                    x: 0.0,
                    y: 12.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr("options.title"),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                out.push(option_widget(s, Opt::Fov, cx - 154.0, 29.0, 150.0));
                out.push(button(
                    cx + 4.0,
                    29.0,
                    150.0,
                    tr("options.online"),
                    Act::Push(Page::Online),
                ));
                let entries = [
                    ("options.skinCustomisation", Some(Page::Skin)),
                    ("options.sounds", Some(Page::Sound)),
                    // Video Settings... opens Horizon's graphics screen, which holds every
                    // video option, as Sodium's replaces vanilla's.
                    ("options.video", Some(Page::Graphics)),
                    ("options.controls", Some(Page::Controls)),
                    (
                        "options.language",
                        (!self.split_guest).then_some(Page::Language),
                    ),
                    ("options.chat", Some(Page::Chat)),
                    (
                        "options.resourcepack",
                        (!self.split_guest).then_some(Page::Packs),
                    ),
                    ("options.accessibility", Some(Page::Accessibility)),
                    // Vanilla's Telemetry Data and Credits slots: Horizon's
                    // settings and its mods, as a mod adds its button here.
                    // Credits & Attribution (WinScreen's credits) follows them.
                    ("horizon.pages.open.horizon_settings", Some(Page::Horizon)),
                    ("horizon.pages.open.mods", Some(Page::Mods)),
                    ("options.credits_and_attribution", Some(Page::Credits)),
                ];
                // horizon-extras: without it, vanilla's last row instead of Horizon Settings and
                // Mods: Telemetry Data (greyed: nothing is sent) and Credits & Attribution.
                let plain;
                let entries: &[_] = if horizon_extras() {
                    &entries
                } else {
                    plain = [
                        &entries[..entries.len() - 3],
                        &[
                            ("options.telemetry", None),
                            ("options.credits_and_attribution", Some(Page::Credits)),
                        ],
                    ]
                    .concat();
                    &plain
                };
                let rows = entries.len().div_ceil(2) as f32;
                let gy = (61.0 + 30.0f32).min(h - 33.0 - rows * 24.0);
                for (i, (key, page)) in entries.iter().enumerate() {
                    let x = cx - 158.0 + 4.0 + (i % 2) as f32 * 158.0;
                    let y = gy + (i / 2) as f32 * 24.0;
                    let b = button(x, y, 150.0, tr(key), page.map_or(Act::None, Act::Push));
                    out.push(if page.is_some() { b } else { inactive(b) });
                }
                out.push(done(h - 27.0));
                top = 0.0;
                bottom = 0.0;
            }
            Page::Credits => {
                // CreditsAndAttributionScreen: the title header, three 210-wide
                // buttons 8 apart (HeaderAndFooterLayout: 30 below the header
                // unless the footer is closer), Done.
                out.push(Widget {
                    x: 0.0,
                    y: 12.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                let y = (33.0 + 30.0f32).min(h - 33.0 - 76.0);
                let x = cx - 105.0;
                out.push(button(
                    x,
                    y,
                    210.0,
                    tr("credits_and_attribution.button.credits"),
                    Act::Credits,
                ));
                // ConfirmLinkScreen to minecraft.net: Horizon opens no links.
                for (i, key) in [
                    "credits_and_attribution.button.attribution",
                    "credits_and_attribution.button.licenses",
                ]
                .into_iter()
                .enumerate()
                {
                    let y = y + 28.0 * (i + 1) as f32;
                    out.push(inactive(button(x, y, 210.0, tr(key), Act::None)));
                }
                // horizon-extras: a build without them says where the official one is.
                if !horizon_extras() {
                    out.push(Widget {
                        x: 0.0,
                        y: (y + 92.0).min(h - 42.0),
                        w,
                        h: 9.0,
                        kind: WKind::Text {
                            text: tr("horizon.plain.official"),
                            color: 0xFFA0A0A0,
                            centered: true,
                        },
                        list: false,
                        row: 0,
                    });
                }
                out.push(done(h - 27.0));
                top = 0.0;
                bottom = 0.0;
            }
            Page::KeyBinds => {
                top = 38.0;
                out.push(Widget {
                    x: 0.0,
                    y: 4.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                let mut mappings: Vec<(&str, &str)> =
                    MAPPINGS.iter().map(|(n, _, c)| (*n, *c)).collect();
                let query = self.settings_search.value.to_lowercase();
                mappings.retain(|(name, _)| {
                    query.is_empty() || lang::get(name).to_lowercase().contains(&query)
                });
                mappings.sort_by(|a, b| {
                    let ca = CATEGORIES.iter().position(|c| *c == a.1).unwrap_or(99);
                    let cb = CATEGORIES.iter().position(|c| *c == b.1).unwrap_or(99);
                    ca.cmp(&cb)
                        .then_with(|| lang::get(a.0).cmp(&lang::get(b.0)))
                });
                let row_left = cx - 170.0;
                let scrollbar_x = row_left + 340.0 + 6.0 + 2.0;
                // VideoSettingsScreen.addTitle: a changed graphics API needs a restart.
                if page == Page::Video && s.horizon.api != s.startup_api {
                    out.push(Widget {
                        x: 0.0,
                        y: 15.0,
                        w,
                        h: 9.0,
                        kind: WKind::Text {
                            text: tr("options.restartRequired"),
                            color: rgba(223, 80, 80, 255),
                            centered: true,
                        },
                        list: false,
                        row: 0,
                    });
                }
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                let mut last_category = "";
                for (name, category) in mappings {
                    if category != last_category {
                        last_category = category;
                        rows.begin(&out, y);
                        let label =
                            Text::translate(&format!("key.category.minecraft.{category}"), &[])
                                .with_style(&Style {
                                    bold: Some(true),
                                    underlined: Some(true),
                                    ..Style::default()
                                });
                        out.push(Widget {
                            x: 0.0,
                            y: y + 7.0,
                            w,
                            h: 9.0,
                            kind: WKind::Text {
                                text: label,
                                color: u32::MAX,
                                centered: true,
                            },
                            list: false,
                            row: 0,
                        });
                        y += 20.0;
                        rows.end(&mut out, y);
                    }
                    rows.begin(&out, y);
                    out.push(Widget {
                        x: row_left + 2.0,
                        y: y + 6.0,
                        w: 200.0,
                        h: 9.0,
                        kind: WKind::Text {
                            text: tr(name),
                            color: u32::MAX,
                            centered: false,
                        },
                        list: false,
                        row: 0,
                    });
                    let key = s.keys.get(name).to_owned();
                    let mut message = keybinds::display_name(&key);
                    let conflicts = key_conflicts(
                        s,
                        "",
                        name,
                        s.keys.get(name),
                        0,
                        crate::mod_list::KeyConflictContext::Universal,
                        s.keys.is_default(name),
                    );
                    if !conflicts.is_empty() {
                        message = Text::literal("[ ")
                            .append(message.with_color(text::WHITE))
                            .append(" ]")
                            .with_color(text::YELLOW);
                    }
                    if self.selected_key == Some(name) {
                        message = Text::literal("> ")
                            .append(message.with_style(&Style {
                                color: Some(text::WHITE),
                                underlined: Some(true),
                                ..Style::default()
                            }))
                            .append(" <")
                            .with_color(text::YELLOW);
                    }
                    let reset_x = scrollbar_x - 50.0 - 10.0;
                    let change_x = reset_x - 5.0 - 75.0;
                    let name_static = MAPPINGS.iter().find(|m| m.0 == name).map_or("", |m| m.0);
                    out.push(button(change_x, y, 75.0, message, Act::Key(name_static)));
                    if !conflicts.is_empty() {
                        out.push(Widget {
                            x: change_x - 6.0,
                            y: y + 1.0,
                            w: 3.0,
                            h: 18.0,
                            kind: WKind::Text {
                                text: Text::empty(),
                                color: rgba(255, 255, 0, 255),
                                centered: false,
                            },
                            list: false,
                            row: 0,
                        });
                    }
                    let reset = button(
                        reset_x,
                        y,
                        50.0,
                        tr("controls.reset"),
                        Act::ResetKey(name_static),
                    );
                    out.push(if s.keys.is_default(name) {
                        inactive(reset)
                    } else {
                        reset
                    });
                    y += 20.0;
                    rows.end(&mut out, y);
                }
                // Each native mod's registered keys, under the mod's title and their categories.
                for (i, m) in s.mods.installed.iter().enumerate() {
                    let Some(page) = m.native.as_ref().filter(|p| !p.mappings.is_empty()) else {
                        continue;
                    };
                    rows.begin(&out, y);
                    out.push(section_title(Text::literal(m.title.clone()), y, w));
                    y += 20.0;
                    rows.end(&mut out, y);
                    let mut last_category = "";
                    for (k, mapping) in page.mappings.iter().enumerate() {
                        if !query.is_empty()
                            && !lang::get(&mapping.label).to_lowercase().contains(&query)
                            && !lang::get(&mapping.category).to_lowercase().contains(&query)
                        {
                            continue;
                        }
                        let category = mapping.category.as_str();
                        if !category.is_empty() && category != last_category {
                            last_category = category;
                            rows.begin(&out, y);
                            out.push(section_title(tr(&mapping.category), y, w));
                            y += 20.0;
                            rows.end(&mut out, y);
                        }
                        let (key, modifiers) = s.mods.binding(
                            &m.id,
                            &mapping.id,
                            (mapping.key.as_str(), mapping.modifiers),
                        );
                        let at_default = key == mapping.key && modifiers == mapping.modifiers;
                        let mut message = binding_text(&key, modifiers);
                        if !key_conflicts(
                            s,
                            &m.id,
                            &mapping.id,
                            &key,
                            modifiers,
                            mapping.context,
                            at_default,
                        )
                        .is_empty()
                        {
                            message = Text::literal("[ ")
                                .append(message.with_color(text::WHITE))
                                .append(" ]")
                                .with_color(text::YELLOW);
                        }
                        if self.selected_mod_key == Some((i, k)) {
                            message = Text::literal("> ")
                                .append(message.with_style(&Style {
                                    color: Some(text::WHITE),
                                    underlined: Some(true),
                                    ..Style::default()
                                }))
                                .append(" <")
                                .with_color(text::YELLOW);
                        }
                        rows.begin(&out, y);
                        out.push(Widget {
                            x: row_left + 2.0,
                            y: y + 6.0,
                            w: 200.0,
                            h: 9.0,
                            kind: WKind::Text {
                                text: tr(&mapping.label),
                                color: u32::MAX,
                                centered: false,
                            },
                            list: false,
                            row: 0,
                        });
                        let reset_x = scrollbar_x - 50.0 - 10.0;
                        let change_x = reset_x - 5.0 - 75.0;
                        out.push(button(change_x, y, 75.0, message, Act::ModKey(i, k)));
                        let reset = button(
                            reset_x,
                            y,
                            50.0,
                            tr("controls.reset"),
                            Act::ResetModKey(i, k),
                        );
                        out.push(if at_default { inactive(reset) } else { reset });
                        y += 20.0;
                        rows.end(&mut out, y);
                    }
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(button(
                    cx - 154.0,
                    h - 27.0,
                    150.0,
                    tr("controls.resetAll"),
                    Act::ResetKeys,
                ));
                out.push(button(cx + 4.0, h - 27.0, 150.0, tr("gui.done"), Act::Done));
            }
            Page::Language => {
                top = 36.0;
                bottom = h - 53.0;
                out.push(Widget {
                    x: 0.0,
                    y: 4.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                self.language_search.x = cx - 100.0;
                self.language_search.y = 17.0;
                let query = self.language_search.value.to_lowercase();
                let current = self
                    .language_choice
                    .clone()
                    .unwrap_or_else(|| s.language.clone());
                if std::mem::take(&mut self.center_language)
                    && let Some(i) = self.languages.iter().position(|l| l.code == current)
                {
                    // AbstractSelectionList.centerScrollOn.
                    self.scroll = (i as f32 * 18.0 + 9.0 - (bottom - top) / 2.0).max(0.0);
                }
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                for l in &self.languages {
                    if !query.is_empty()
                        && !l.label().to_lowercase().contains(&query)
                        && !l.code.contains(&query)
                    {
                        continue;
                    }
                    rows.begin(&out, y);
                    out.push(Widget {
                        x: cx - 110.0,
                        y,
                        w: 220.0,
                        h: 18.0,
                        kind: WKind::LanguageRow {
                            label: Text::literal(l.label()),
                            selected: l.code == current,
                            code: l.code.clone(),
                        },
                        list: false,
                        row: 0,
                    });
                    y += 18.0;
                    rows.end(&mut out, y);
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                out.push(Widget {
                    x: 0.0,
                    y: h - 45.0,
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr("options.languageAccuracyWarning"),
                        color: rgba(0xBA, 0xBA, 0xBA, 255),
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                out.push(button(
                    cx - 154.0,
                    h - 28.0,
                    150.0,
                    tr("options.font"),
                    Act::Push(Page::Font),
                ));
                out.push(button(cx + 4.0, h - 28.0, 150.0, tr("gui.done"), Act::Done));
            }
            // Drawn by `pack_screen` and `mods_screen`.
            Page::Packs | Page::Mods => {}
            _ => {
                if matches!(page, Page::Video | Page::Controls | Page::ModBrowser) {
                    top = 38.0;
                }
                out.push(Widget {
                    x: 0.0,
                    y: if matches!(page, Page::Video | Page::Controls | Page::ModBrowser) {
                        4.0
                    } else {
                        12.0
                    },
                    w,
                    h: 9.0,
                    kind: WKind::Text {
                        text: tr(Self::title(page)),
                        color: u32::MAX,
                        centered: true,
                    },
                    list: false,
                    row: 0,
                });
                let list_start = out.len();
                let mut y = top + 2.0 - self.scroll;
                for row in self.list_rows(s) {
                    rows.begin(&out, y);
                    match row {
                        Row::Header(key) => {
                            let label = tr(key);
                            self.heading_row(&mut out, label, &mut y, w, top);
                        }
                        Row::Heading(key) => {
                            let label = tr(&key);
                            self.heading_row(&mut out, label, &mut y, w, top);
                        }
                        Row::Note(text) => {
                            out.push(Widget {
                                x: 0.0,
                                y: y + 4.0,
                                w,
                                h: 9.0,
                                kind: WKind::Text {
                                    text: Text::literal(text),
                                    color: 0xFFA0A0A0,
                                    centered: true,
                                },
                                list: false,
                                row: 0,
                            });
                            y += 16.0;
                        }
                        Row::Big(opt) => {
                            out.push(option_widget(s, opt, cx - 155.0, y + 2.0, 310.0));
                            y += 25.0;
                        }
                        Row::Pair(a, b) => {
                            out.push(option_widget(s, a, cx - 155.0, y + 2.0, 150.0));
                            if let Some(b) = b {
                                out.push(option_widget(s, b, cx + 5.0, y + 2.0, 150.0));
                            }
                            y += 25.0;
                        }
                        Row::Buttons(mut a, b) => {
                            a.x = cx - 155.0;
                            a.y = y + 2.0;
                            out.push(a);
                            if let Some(mut b) = b {
                                b.x = cx + 5.0;
                                b.y = y + 2.0;
                                out.push(b);
                            }
                            y += 25.0;
                        }
                    }
                    rows.end(&mut out, y);
                }
                content = y + self.scroll - top + 2.0;
                out[list_start..].iter_mut().for_each(|w| w.list = true);
                let folder = match page {
                    Page::Accessibility => {
                        Some(("options.accessibility.link", Act::AccessibilityHelp))
                    }
                    _ => None,
                };
                if let Some((label, act)) = folder {
                    out.push(button(cx - 154.0, h - 27.0, 150.0, tr(label), act));
                    out.push(button(cx + 4.0, h - 27.0, 150.0, tr("gui.done"), Act::Done));
                } else {
                    out.push(done(h - 27.0));
                }
            }
        }
        // Layout may change the offset (language centering), or the list may
        // shrink after a resize/search. Correct the rows in this same frame.
        let previous_scroll = self.scroll;
        self.clamp_scroll(top, bottom, content);
        for widget in &mut out {
            if widget.list {
                widget.y += previous_scroll - self.scroll;
            }
            if let WKind::Button {
                act: Act::Cycle(Opt::RotateWithMinecarts),
                active,
                ..
            } = &mut widget.kind
            {
                *active = self.in_world && self.minecart_improvements;
            }
        }
        self.row_bands = rows
            .bands
            .into_iter()
            .map(|band| band.map(|y| y + previous_scroll - self.scroll))
            .collect();
        (out, top, bottom, content)
    }
}
