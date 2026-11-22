//! A frame's render continuation. Each stage releases OptionsUi before a host hook runs.
use super::*;
use crate::screen_draw::Draw;

/// Stable agent handles for the options menu actions, not for their captions.
fn button_id(page: Page, act: &Act) -> Option<&'static str> {
    match (page, act) {
        (_, Act::Done) => Some("done"),
        (_, Act::Cycle(opt)) => opt.agent_id(),
        (Page::Horizon, Act::Push(Page::Agent)) => Some("ai-agent"),
        (Page::Main, Act::Push(page)) => Some(match page {
            Page::Online => "online",
            Page::Skin => "skin-customization",
            Page::Sound => "music-sounds",
            Page::Graphics => "video-settings",
            Page::Controls => "controls",
            Page::Language => "language",
            Page::Chat => "chat-settings",
            Page::Packs => "resource-packs",
            Page::Accessibility => "accessibility-settings",
            Page::Horizon => "horizon-settings",
            Page::Mods => "mods",
            Page::Credits => "credits-attribution",
            _ => return None,
        }),
        (Page::Main, Act::None) if !horizon_extras() => Some("telemetry-data"),
        _ => None,
    }
}

pub(super) enum FrameDraw {
    Error,
    Calibration(crate::hdr_calibration::Layout),
    Shell(Page),
    RadialName,
    SplitSetup,
    SplitProfiles,
    Studio([f32; 4]),
    Mods,
    Packs,
    Standard(std::sync::Arc<OptionsLayout>),
    Tooltip(Option<Vec<Text>>, Option<[f32; 4]>),
    Label(Option<String>),
    SplitHelp(Option<String>),
    Draws(Vec<Draw>, Option<usize>),
    Complete,
}
impl OptionsUi {
    /// Discard a cancelled frame, including the hosted shell's prepared view.
    pub fn discard_draw(&mut self) {
        if let Some(FrameDraw::Shell(page)) = self.frame_draw.take() {
            let hosted: &mut dyn shell::Hosted = match page {
                Page::Extras => &mut self.extras,
                page if Self::graphics_page(page).is_some() => &mut self.graphics,
                Page::ControllerRadial => &mut self.radial,
                Page::ControllerRadialPick => &mut self.radial_pick,
                page if Self::controller_page(page).is_some() => &mut self.controller,
                page if Self::shader_page(page).is_some() => &mut self.shader,
                Page::PackOptions => &mut self.pack_options,
                _ => unreachable!("prepared shell page"),
            };
            hosted.discard_draw();
        }
    }
    /// Paint the background and prepare this frame exactly once. No borrow survives this call.
    pub fn draw_background(
        &mut self,
        s: &Settings,
        output: crate::gfx::EffectiveOutput,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
    ) {
        self.discard_draw();
        self.output = output.shown(self.in_world);
        let (w, h) = (batch.width, batch.height);
        let page = self.page();
        if self.resource_error.is_some() {
            ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.in_world);
            self.frame_draw = Some(FrameDraw::Error);
            return;
        }
        crate::widgets::begin_screen(|| {
            crate::widgets::ScreenInfo::new(
                format!("options/{page:?}"),
                tr(Self::title(page)).plain(),
            )
        });
        let waiting = self.waiting();
        let in_world = self.in_world;
        self.radial.screen.glyphs = self.pad_glyphs;
        let frame = if page == Page::HdrCalibration
            && let Some(calibration) = self.calibration.as_ref()
        {
            FrameDraw::Calibration(calibration.draw_background(batch, atlas))
        } else if page == Page::ControllerRadialName && self.radial.screen.naming.is_some() {
            ui::menu_background(batch, atlas, 0.0, 0.0, w.ceil(), h.ceil(), in_world);
            FrameDraw::RadialName
        } else if let Some(shell) = self.shell_mut() {
            shell.draw_background(s, batch, atlas, in_world, waiting);
            self.size = (w, h);
            FrameDraw::Shell(page)
        } else if page == Page::Studio {
            FrameDraw::Studio(self.studio.draw_background(batch, atlas, in_world))
        } else if matches!(
            page,
            Page::SplitSetup | Page::SplitProfiles | Page::Mods | Page::Packs
        ) {
            if page == Page::Packs {
                self.watch_data_packs(atlas, (w, h));
            }
            let (bw, bh) = if page == Page::SplitProfiles {
                (w.ceil(), h.ceil())
            } else {
                (w, h)
            };
            ui::menu_background(batch, atlas, 0.0, 0.0, bw, bh, in_world);
            match page {
                Page::SplitSetup => {
                    self.split_setup.on = s.extras.splitscreen;
                    self.split_setup.size = (w, h);
                    FrameDraw::SplitSetup
                }
                Page::SplitProfiles => FrameDraw::SplitProfiles,
                Page::Mods => FrameDraw::Mods,
                Page::Packs => FrameDraw::Packs,
                _ => unreachable!(),
            }
        } else {
            if page == Page::Pvp {
                self.wide_extras = wide_extras(s, |t| atlas.font.width(t));
            }
            let layout = self.built_layout(s, w, h);
            let (_, top, bottom, _) = layout.parts();
            self.size = (w, h);
            ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
            if bottom > top {
                ui::list_background(batch, atlas, top, bottom, in_world);
            }
            FrameDraw::Standard(layout)
        };
        self.frame_draw = Some(frame);
    }
    /// Paint contents only; retain existing tooltip data for the final stage.
    pub fn draw_contents(
        &mut self,
        s: &Settings,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
    ) {
        let frame = self
            .frame_draw
            .take()
            .expect("options background before contents");
        let next = match frame {
            FrameDraw::Error => {
                self.draw_resource_error(batch, atlas, cursor);
                FrameDraw::Complete
            }
            FrameDraw::Calibration(layout) => {
                self.calibration
                    .as_ref()
                    .expect("calibration remains open")
                    .draw_contents(batch, atlas, cursor, layout);
                FrameDraw::Complete
            }
            FrameDraw::Shell(_) => {
                self.shell_mut()
                    .expect("shell remains open")
                    .draw_contents(batch, atlas, cursor);
                FrameDraw::Complete
            }
            FrameDraw::RadialName => {
                self.radial
                    .screen
                    .naming
                    .as_mut()
                    .expect("name entry remains open")
                    .draw_contents(batch, atlas, cursor);
                FrameDraw::Complete
            }
            FrameDraw::SplitSetup => {
                FrameDraw::SplitHelp(self.split_setup.draw_contents(batch, atlas, cursor))
            }
            FrameDraw::SplitProfiles => FrameDraw::Label(self.split_profiles.draw_contents(
                batch,
                atlas,
                cursor,
                self.pad_glyphs,
            )),
            FrameDraw::Studio(rect) => {
                self.studio.draw_contents(batch, atlas, cursor, rect);
                FrameDraw::Complete
            }
            kind @ (FrameDraw::Mods | FrameDraw::Packs) => {
                let size = (batch.width, batch.height);
                let blink = self.pack_focus.elapsed().as_millis() as u64;
                let m = crate::screen_draw::Measure(&atlas.font);
                let mouse = (cursor.0.floor() as i32, cursor.1.floor() as i32);
                let draws = if matches!(kind, FrameDraw::Mods) {
                    let screen = self.mods_page(s, size);
                    let mut draws = screen.draw(&m, mouse, blink, crate::hud_overlays::now_ms());
                    screen.bind_icons(&mut draws, atlas, &mut batch.patches);
                    draws
                } else {
                    let screen = self.packs(atlas, size);
                    let mut draws = screen.draw(&m, mouse, blink, crate::hud_overlays::now_ms());
                    screen.bind_icons(&mut draws, atlas, &mut batch.patches);
                    draws
                };
                let tooltip = crate::screen_draw::paint(batch, atlas, &draws);
                FrameDraw::Draws(draws, tooltip)
            }
            FrameDraw::Standard(layout) => {
                self.draw_standard_contents(s, batch, atlas, cursor, &layout)
            }
            _ => panic!("options contents drawn exactly once"),
        };
        self.frame_draw = Some(next);
    }
    /// Paint deferred overlays and consume this frame's continuation.
    pub fn draw_overlays(&mut self, batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32)) {
        match self
            .frame_draw
            .take()
            .expect("options contents before overlays")
        {
            FrameDraw::Complete => {}
            FrameDraw::Tooltip(lines, rect) => {
                if let Some(lines) = lines {
                    match rect {
                        Some(rect) if self.pad_driving => {
                            ui::tooltip_under(batch, atlas, &lines, rect)
                        }
                        Some(rect) => ui::tooltip_menu(batch, atlas, &lines, cursor, rect),
                        None => ui::tooltip(batch, atlas, &lines, cursor),
                    }
                }
            }
            FrameDraw::Label(tip) => {
                if let Some(tip) = tip {
                    ui::tooltip(batch, atlas, &[Text::literal(tip)], cursor);
                }
            }
            FrameDraw::SplitHelp(tip) => {
                if let Some(tip) = tip {
                    ui::tooltip(
                        batch,
                        atlas,
                        &atlas.font.split(&Text::literal(tip), 170.0),
                        cursor,
                    );
                }
            }
            FrameDraw::Draws(draws, tooltip) => {
                crate::screen_draw::paint_tooltip(batch, atlas, &draws, tooltip)
            }
            _ => panic!("options overlays follow contents"),
        }
    }
    #[cfg(test)]
    pub(super) fn draw_test_frame(
        &mut self,
        s: &Settings,
        output: crate::gfx::EffectiveOutput,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        phases: &mut dyn FnMut(crate::screen_draw::RenderPhase, &mut HudBatch),
    ) {
        self.draw_background(s, output, batch, atlas);
        phases(crate::screen_draw::RenderPhase::Background, batch);
        self.draw_contents(s, batch, atlas, cursor);
        phases(crate::screen_draw::RenderPhase::Foreground, batch);
        self.draw_overlays(batch, atlas, cursor);
    }
    fn draw_resource_error(&self, batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32)) {
        let error = self
            .resource_error
            .as_ref()
            .expect("resource error remains open");
        let (w, h) = (batch.width, batch.height);
        let title = tr("resourcePack.load_fail");
        // An alert: its title and its message.
        crate::widgets::begin_screen(|| {
            crate::widgets::ScreenInfo::new(
                "options/resource_error",
                crate::widgets::join_for_narration(&[&title.plain(), error]),
            )
        });
        batch.text_styled(
            atlas,
            &title,
            ((w - atlas.font.width_text(&title)) / 2.0).floor(),
            30.0,
            u32::MAX,
            true,
        );
        let lines = atlas.font.split(&Text::literal(error), (w - 50.0).max(1.0));
        let limit = ((h - 110.0) / 9.0).max(1.0) as usize;
        for (i, line) in lines.iter().take(limit).enumerate() {
            batch.text_styled(
                atlas,
                line,
                ((w - atlas.font.width_text(line)) / 2.0).floor(),
                60.0 + i as f32 * 9.0,
                u32::MAX,
                true,
            );
        }
        ui::action_button(
            batch,
            atlas,
            crate::widgets::Action::Done,
            w / 2.0 - 100.0,
            h - 28.0,
            200.0,
            20.0,
            &tr("gui.done"),
            true,
            ui::hovered(cursor, w / 2.0 - 100.0, h - 28.0, 200.0, 20.0),
        );
    }
    fn draw_standard_contents(
        &mut self,
        s: &Settings,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        layout: &OptionsLayout,
    ) -> FrameDraw {
        let (w, h) = (batch.width, batch.height);
        let page = self.page();
        let in_world = self.in_world;
        let (widgets, top, bottom, content) = layout.parts();
        let list = bottom > top;
        let visible = |wd: &Widget| !wd.list || (wd.y < bottom && wd.y + wd.h > top);
        // The list's narration: the language list is an ObjectSelectionList,
        // the others ContainerObjectSelectionLists whose rows' widgets are
        // recorded as they are drawn.
        let list_key = crate::widgets::key_of(&format!("options/{page:?}/list"));
        let framebuffer = [(w * batch.scale) as i32, (h * batch.scale) as i32];
        if list {
            self.narrate_list(widgets, list_key, (top, bottom), w, cursor);
        }
        let mut tooltip = None;
        // The widget a keyboard-focused tooltip belongs to: `MenuTooltipPositioner`
        // keeps it against that widget, not at the pointer.
        let mut tooltip_rect: Option<[f32; 4]> = None;
        for (wd_idx, wd) in widgets.iter().enumerate() {
            if !visible(wd) {
                continue;
            }
            let is_focused = self
                .focus
                .as_ref()
                .is_some_and(|fid| matches_focus(fid, wd_idx, wd));
            let start = batch.vertices.len();
            let pointer_over = ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h)
                && (!wd.list || cursor.1 >= top && cursor.1 < bottom);
            let hover = pointer_over || is_focused;
            // The tooltip is the pointer's (`setTooltip`, drawn while hovered);
            // a keyboard-focused widget keeps it against the widget
            // (`MenuTooltipPositioner`). Focus a mouse click leaves behind
            // does not: the description goes when the mouse leaves, like
            // vanilla's.
            let tip_hover = pointer_over || (is_focused && self.focus_from_keyboard);
            if tip_hover && let Some(tip) = tooltip_text(s, wd) {
                tooltip = Some(match &wd.kind {
                    // The duplicate keybinds tooltip keeps its own lines.
                    WKind::Button {
                        act: Act::Key(_), ..
                    } => tip
                        .plain()
                        .split('\n')
                        .map(|l| Text::literal(l.to_owned()))
                        .collect::<Vec<_>>(),
                    _ => atlas.font.split(&tip, 170.0),
                });
                tooltip_rect =
                    (!pointer_over || self.pad_driving).then_some([wd.x, wd.y, wd.w, wd.h]);
            }
            match &wd.kind {
                WKind::Button { label, active, act } => {
                    ui::focused_button(
                        batch,
                        atlas,
                        [wd.x, wd.y, wd.w, wd.h],
                        label,
                        *active,
                        pointer_over,
                        is_focused,
                        matches!(act, Act::Done).then_some(crate::widgets::Action::Done),
                        button_id(page, act),
                    );
                    crate::widgets::last_narratable(|n| {
                        narrate_widget(n, s, wd, hover, list_key, framebuffer);
                    });
                }
                WKind::Slider { label, value, opt } => {
                    let dragging = self.dragging == Some(*opt);
                    let shown = self.slider_shown(*opt, *value);
                    ui::slider_ext(
                        batch,
                        atlas,
                        wd.x,
                        wd.y,
                        wd.w,
                        label,
                        shown,
                        hover || dragging,
                        is_focused,
                        self.can_change_value && is_focused,
                        true,
                        dragging,
                        opt.agent_id(),
                    );
                    crate::widgets::last_narratable(|n| {
                        narrate_widget(n, s, wd, hover, list_key, framebuffer);
                    });
                }
                WKind::Text {
                    text,
                    color,
                    centered,
                } => {
                    if text.is_empty() {
                        batch.fill(atlas, wd.x, wd.y, wd.w, wd.h, *color);
                    } else if *centered {
                        let tw = atlas.font.width_text(text);
                        // A list's header row (OptionsList.HeaderEntry,
                        // KeyBindsList.CategoryEntry): a FocusableTextWidget
                        // narrating its text, 4 pixels of padding round it.
                        if wd.list && *color == u32::MAX {
                            crate::widgets::narratable(|| {
                                let x = wd.x + ((wd.w - tw) / 2.0).floor();
                                crate::widgets::Narratable::text(
                                    text.plain(),
                                    [x - 4.0, wd.y - 4.0, tw + 8.0, 17.0],
                                )
                                .key(crate::widgets::key_of(&(list_key, wd.row)))
                                .child_of(list_key, wd.row)
                            });
                        }
                        batch.text_styled(
                            atlas,
                            text,
                            wd.x + ((wd.w - tw) / 2.0).floor(),
                            wd.y,
                            *color,
                            true,
                        );
                    } else {
                        batch.text_styled(atlas, text, wd.x, wd.y, *color, true);
                    }
                }
                WKind::LanguageRow {
                    label, selected, ..
                } => {
                    let rect = [wd.x, wd.y, wd.w, wd.h];
                    crate::widgets::record_as(
                        &label.plain(),
                        rect,
                        true,
                        crate::widgets::Kind::Row,
                    );
                    crate::widgets::target_as(rect, crate::widgets::Kind::Row);
                    if *selected || is_focused {
                        batch.fill(atlas, wd.x, wd.y, wd.w, wd.h, u32::MAX);
                        batch.fill(
                            atlas,
                            wd.x + 1.0,
                            wd.y + 1.0,
                            wd.w - 2.0,
                            wd.h - 2.0,
                            rgba(0, 0, 0, 255),
                        );
                    }
                    let tw = atlas.font.width_text(label);
                    batch.text_styled(
                        atlas,
                        label,
                        wd.x + ((wd.w - tw) / 2.0).floor(),
                        wd.y + 2.0 + 7.0 - 4.0,
                        u32::MAX,
                        true,
                    );
                }
            }
            if wd.list && (wd.y < top || wd.y + wd.h > bottom) {
                batch.clip_since(start, 0.0, top, w, bottom);
            }
        }
        if list {
            ui::separators(batch, atlas, top, bottom, in_world);
            let sx = self.scrollbar_x(w);
            ui::scrollbar(batch, atlas, sx, top, bottom, content, self.scroll);
            // AbstractScrollArea.extractScrollbar: over a scrollable bar, the
            // hand, or the vertical resize while it is dragged.
            if ui::scrollbar_thumb(top, bottom, content, self.scroll).is_some()
                && ui::hovered(cursor, sx, top, 6.0, bottom - top)
            {
                crate::widgets::request_cursor(if self.scrollbar_drag_y.is_some() {
                    crate::widgets::Cursor::ResizeNs
                } else {
                    crate::widgets::Cursor::PointingHand
                });
            }
        }
        if self.page() == Page::Language {
            let mut search = self.language_search.clone();
            if self.focus == Some(FocusId::Search) {
                search.focused = true;
            }
            search.draw(batch, atlas, None);
            // LanguageSelectScreen's header search box (no message), added
            // before the list.
            crate::widgets::last_narratable(|n| n.order = -1);
            if search.value.is_empty() && !search.focused {
                batch.text_styled(
                    atlas,
                    &tr("gui.language.search").with_color(text::GRAY).italic(),
                    search.text_x(),
                    search.text_y(),
                    u32::MAX,
                    true,
                );
            }
            self.language_search = search;
        }
        if matches!(
            self.page(),
            Page::Video | Page::Controls | Page::KeyBinds | Page::ModBrowser
        ) {
            let mut search = self.settings_search.clone();
            if self.focus == Some(FocusId::Search) {
                search.focused = true;
            }
            search.draw(batch, atlas, None);
            let hint = match self.page() {
                Page::ModBrowser => "horizon.mods.browse.search",
                _ => "horizon.options.search",
            };
            // Horizon's search box sits in the header, before the list.
            crate::widgets::last_narratable(|n| {
                n.order = -1;
                if let crate::widgets::What::EditBox { message, .. } = &mut n.what {
                    *message = tr(hint).plain();
                }
            });
            if search.value.is_empty() && !search.focused {
                batch.text_styled(
                    atlas,
                    &tr(hint).with_color(text::GRAY).italic(),
                    search.text_x(),
                    search.text_y(),
                    u32::MAX,
                    true,
                );
            }
            self.settings_search = search;
        }
        FrameDraw::Tooltip(tooltip, tooltip_rect)
    }
}
