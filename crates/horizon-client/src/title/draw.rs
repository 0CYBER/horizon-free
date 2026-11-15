use super::*;

impl Title {
    pub fn draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        hide_splash: bool,
        phases: &mut dyn FnMut(crate::screen_draw::RenderPhase, &mut HudBatch),
    ) {
        self.draw_with_items(atlas, None, batch, cursor, hide_splash, false, phases);
    }
    /// [`Title::draw`] with the item models, which draw the item icons of the
    /// Superflat screens the way `GuiGraphics.renderItem` does.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_with_items(
        &self,
        atlas: &HudAtlas,
        items: Option<&crate::item_render::ItemRenderer>,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        hide_splash: bool,
        hide_server_address: bool,
        phases: &mut dyn FnMut(crate::screen_draw::RenderPhase, &mut HudBatch),
    ) {
        let (w, h) = (batch.width.ceil(), batch.height.ceil());
        if self.rules.borrow().is_some() {
            self.rules
                .borrow()
                .as_ref()
                .expect("rules screen is open")
                .draw_background(batch, atlas);
            phases(crate::screen_draw::RenderPhase::Background, batch);
            let tooltip = self
                .rules
                .borrow_mut()
                .as_mut()
                .expect("rules screen is open")
                .draw_contents(batch, atlas, cursor);
            phases(crate::screen_draw::RenderPhase::Foreground, batch);
            if let Some(tooltip) = tooltip {
                crate::ui::tooltip(batch, atlas, &tooltip, cursor);
            }
            return;
        }
        if self.preparing {
            // `CreateWorldScreen.openCreateWorldScreen`'s queued
            // `GenericMessageScreen`: the menu background and the message
            // alone, centred, no progress.
            crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
            phases(crate::screen_draw::RenderPhase::Background, batch);
            let text =
                crate::lang::get_or("createWorld.preparing", "Preparing for world creation...");
            let tw = atlas.font.width_text(&Text::literal(text.clone()));
            let (tx, ty) = (w / 2.0 - tw / 2.0, h / 2.0 - 4.5);
            // `FocusableTextWidget.builder(title, font, 12)`: 12 pixels of
            // padding around the text, on a black box with a 1-pixel border.
            let (bx, by) = (tx - 12.0, ty - 12.0);
            let (bw, bh) = (tw + 24.0, 9.0 + 24.0);
            batch.fill(atlas, bx - 1.0, by - 1.0, bw + 2.0, bh + 2.0, 0xFFD0D0D0);
            batch.fill(atlas, bx, by, bw, bh, 0xFF000000);
            batch.text(atlas, &text, tx, ty, u32::MAX, false);
            phases(crate::screen_draw::RenderPhase::Foreground, batch);
            return;
        }
        if let Some(screen) = self.confirm_experiments.borrow().as_ref() {
            return screen.draw(batch, atlas, cursor, phases);
        }
        if let Some(screen) = self.experiments.borrow().as_ref() {
            return screen.draw(batch, atlas, cursor, phases);
        }
        let mut fade_from = None;
        if crate::widgets::narrating()
            && let Some(info) = self.narration()
        {
            crate::widgets::begin_screen(|| info);
        }
        match &self.mode {
            Mode::Main => {
                phases(crate::screen_draw::RenderPhase::Background, batch);
                draw_logo(atlas, batch, w);
                if !hide_splash && let Some(splash) = self.splash.as_deref() {
                    let tw = HudBatch::text_width(atlas, splash);
                    let start = batch.vertices.len();
                    batch.text(
                        atlas,
                        splash,
                        -(tw / 2.0).floor(),
                        -8.0,
                        rgba(255, 255, 0, 255),
                        true,
                    );
                    let scale = (1.8
                        - (self.started.elapsed().as_secs_f32() * std::f32::consts::TAU)
                            .sin()
                            .abs()
                            * 0.1)
                        * 100.0
                        / (tw + 32.0);
                    let (sin, cos) = (-20.0f32).to_radians().sin_cos();
                    for v in &mut batch.vertices[start..] {
                        let [x, y] = v.pos;
                        v.pos = [
                            w / 2.0 + 123.0 + scale * (x * cos - y * sin),
                            69.0 + scale * (x * sin + y * cos),
                        ];
                    }
                }
                let version = format!("Horizon {} (Rust)", horizon_core::VANILLA_VERSION);
                batch.text(atlas, &version, 2.0, h - 10.0, u32::MAX, true);
                // A nightly names its build in the top-left corner, the one spot no button reaches at any GUI
                // scale, so screenshots and reports say which build they come from.
                if let Some(nightly) = horizon_core::release::nightly_label() {
                    batch.text(atlas, &nightly, 2.0, 2.0, u32::MAX, true);
                }
                let notice = &lang::get_or(
                    "horizon.title.not_affiliated",
                    "Not affiliated with Mojang or Microsoft",
                );
                // TitleScreen.init's `PlainTextButton`: the text's width wide, 10
                // high at `height - 10`; `extractContents` underlines it while
                // hovered or focused, and its press opens the credits.
                let tw = HudBatch::text_width(atlas, notice).ceil();
                let rect = [w - tw - 2.0, h - 10.0, tw, 10.0];
                self.credits_button.set(rect);
                crate::widgets::record(notice, rect, true);
                let inside = cursor.0 >= rect[0]
                    && cursor.0 < rect[0] + rect[2]
                    && cursor.1 >= rect[1]
                    && cursor.1 < rect[1] + rect[3];
                crate::widgets::widget_cursor(inside, true);
                let mut text = Text::literal(notice);
                if inside {
                    text = text.underlined();
                }
                batch.text_styled(atlas, &text, rect[0], rect[1], u32::MAX, true);
            }
            Mode::Select { worlds, selected } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                centered(
                    atlas,
                    batch,
                    &lang::get_or("selectWorld.title", "Select World"),
                    8.0,
                    u32::MAX,
                );
                self.caret.borrow_mut().draw(
                    batch,
                    atlas,
                    (SEARCH, &self.search, 50),
                    [(w / 2.0).floor() - 100.0, 21.0, 200.0, 20.0],
                    self.search_focused,
                    self.started,
                );
                crate::widgets::name_last_edit_box(|| narration_text("selectWorld.search", &[]));
                if self.search.is_empty() && !self.search_focused {
                    batch.text_styled(
                        atlas,
                        &Text::literal(lang::get_or("gui.selectWorld.search", "Search..."))
                            .italic()
                            .with_color(crate::text::GRAY),
                        (w / 2.0).floor() - 96.0,
                        27.0,
                        u32::MAX,
                        true,
                    );
                }
                let (top, bottom, left, content) = self.world_list(worlds, (w, h));
                let mut scroll = self.scroll.get();
                scroll.clamp(top, bottom, content);
                self.scroll.set(scroll);
                let offset = scroll.amount.trunc();
                // AbstractSelectionList: background, rows clipped to the
                // list, separators, scrollbar.
                crate::ui::list_background(batch, atlas, top, bottom, false);
                let first = batch.vertices.len();
                // Entry content sits 2 pixels inside its row.
                let x = left + 2.0;
                let mut slot = 0;
                let hovered = self.world_row_at(worlds, cursor, (w, h));
                // WorldSelectionList: the row under the mouse, else the
                // selected one.
                crate::widgets::narratable(|| {
                    use crate::widgets::{ListKind, Narratable, Row};
                    let rows: Vec<&WorldEntry> =
                        self.visible_worlds(worlds).map(|(_, w)| w).collect();
                    let row = |r: usize| Row::new(r, world_narration(rows[r]));
                    let selected = self
                        .visible_worlds(worlds)
                        .position(|(i, _)| Some(i) == *selected);
                    Narratable::list(
                        ListKind::Object,
                        rows.len(),
                        hovered.map(|(r, _)| row(r)),
                        selected.map(row),
                        [0.0, top, w, bottom - top],
                    )
                    .key(crate::widgets::key_of(&"select_world/worlds"))
                });
                for (row, (i, world)) in self.visible_worlds(worlds).enumerate() {
                    let y = top + 4.0 + row as f32 * ROW_HEIGHT - offset;
                    if y + 32.0 <= top || y >= bottom {
                        continue;
                    }
                    let mid = y - 2.0 + ROW_HEIGHT / 2.0;
                    if mid > top && mid < bottom {
                        let rect = [x - 2.0, y - 2.0, ROW_WIDTH, ROW_HEIGHT];
                        // The control socket's widget tree names the row.
                        crate::widgets::record_as(
                            &world.name,
                            rect,
                            true,
                            crate::widgets::Kind::Row,
                        );
                        crate::widgets::target_as(rect, crate::widgets::Kind::Row);
                    }
                    // extractSelection: white once the list has focus.
                    if *selected == Some(i) {
                        batch.fill(atlas, x - 2.0, y - 2.0, ROW_WIDTH, ROW_HEIGHT, u32::MAX);
                        batch.fill(
                            atlas,
                            x - 1.0,
                            y - 1.0,
                            ROW_WIDTH - 2.0,
                            ROW_HEIGHT - 2.0,
                            rgba(0, 0, 0, 255),
                        );
                    }
                    batch.text(atlas, &world.name, x + 35.0, y + 1.0, u32::MAX, true);
                    let date: chrono::DateTime<chrono::Local> = world.modified.into();
                    let line = format!("{} ({})", world.folder, date.format("%d/%m/%Y, %H:%M"));
                    batch.text(
                        atlas,
                        &line,
                        x + 35.0,
                        y + 12.0,
                        rgba(128, 128, 128, 255),
                        true,
                    );
                    let details = world_info(world);
                    batch.text_styled(
                        atlas,
                        &details,
                        x + 35.0,
                        y + 21.0,
                        rgba(128, 128, 128, 255),
                        true,
                    );
                    self.icon(atlas, batch, world, slot, x, y);
                    slot = (slot + 1) % crate::hud::WORLD_ICON_SLOTS;
                    // WorldListEntry.extractContent: a hovered row dims its
                    // icon under the join arrow, lit under the pointer.
                    if let Some((_, (rx, ry))) = hovered.filter(|(r, _)| *r == row) {
                        batch.fill(atlas, x, y, 32.0, 32.0, 0xA0909090);
                        let on_icon = (0.0..32.0).contains(&rx) && (0.0..32.0).contains(&ry);
                        // The icon says what a click does.
                        let names: &[&str] = if world.corrupted {
                            &["error", "marked_join"]
                        } else if world.disabled() {
                            &["error"]
                        } else if world.downgrade {
                            &["marked_join", "error"]
                        } else if world.should_backup() {
                            &["marked_join"]
                        } else {
                            &["join"]
                        };
                        let lit = if on_icon { "_highlighted" } else { "" };
                        for name in names {
                            let sprite = format!("gui/sprites/world_list/{name}{lit}");
                            batch.sprite_scaled(atlas, &sprite, x, y, 32.0, 32.0, u32::MAX);
                        }
                    }
                }
                // Clip list quads, including their UVs, to the content area.
                for quad in batch.vertices[first..].as_chunks_mut::<6>().0 {
                    let low = quad.iter().map(|v| v.pos[1]).fold(f32::INFINITY, f32::min);
                    let high = quad
                        .iter()
                        .map(|v| v.pos[1])
                        .fold(f32::NEG_INFINITY, f32::max);
                    let uv_top = quad.iter().map(|v| v.uv[1]).fold(f32::INFINITY, f32::min);
                    let uv_bot = quad
                        .iter()
                        .map(|v| v.uv[1])
                        .fold(f32::NEG_INFINITY, f32::max);
                    for v in quad {
                        v.pos[1] = v.pos[1].clamp(top, bottom);
                        if high > low {
                            v.uv[1] = uv_top + (uv_bot - uv_top) * (v.pos[1] - low) / (high - low);
                        }
                    }
                }
                crate::ui::separators(batch, atlas, top, bottom, false);
                crate::ui::scrollbar(
                    batch,
                    atlas,
                    left + ROW_WIDTH + 8.0,
                    top,
                    bottom,
                    content,
                    scroll.amount,
                );
            }
            Mode::Create {
                name,
                seed,
                focus,
                options,
                ..
            } => self.draw_create(atlas, batch, cursor, name, seed, *focus, options, phases),
            Mode::CreateFlat { flat, .. } => {
                flat::draw(self, atlas, items, batch, cursor, flat, phases)
            }
            Mode::CreateBuffet { buffet, .. } => {
                buffet::draw(self, atlas, batch, cursor, buffet, phases)
            }
            Mode::CreatePreset { .. } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
            }
            Mode::Edit { name, focused, .. } => {
                self.draw_edit(atlas, batch, (w, h), name, *focused, phases)
            }
            Mode::AdoptBiomeRegions { .. } => self.draw_adopt_regions(atlas, batch, (w, h), phases),
            Mode::Delete { world } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                let message = translate(
                    "selectWorld.deleteWarning",
                    "'%s' will be lost forever! (A long time!)",
                    &[&world.name],
                );
                let lines = wrapped(atlas, &message, w - 50.0);
                self.message_lines.set(lines.len());
                let top = confirm_top(h, lines.len());
                centered(
                    atlas,
                    batch,
                    &lang::get_or(
                        "selectWorld.deleteQuestion",
                        "Are you sure you want to delete this world?",
                    ),
                    top,
                    u32::MAX,
                );
                for (i, line) in lines.iter().enumerate() {
                    centered(atlas, batch, line, top + 17.0 + i as f32 * 9.0, u32::MAX);
                }
            }
            Mode::Recover { world, issues } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                let dir = self.saves.join(&world.folder);
                let title =
                    Text::literal(lang::get_or("recover_world.title", "Failed to load world"))
                        .bold();
                let message = format_translation(
                    &lang::get_or(
                        "recover_world.message",
                        "The following issues occurred while trying to read world folder \"%s\".\nIt might be possible to restore the world from an older state or you can report this issue on the bug tracker.",
                    ),
                    &[Text::colored(world.folder.clone(), GRAY)],
                );
                let message: Vec<Text> = atlas
                    .font
                    .split_lines(&message, w - 50.0)
                    .into_iter()
                    .map(|(line, _)| line)
                    .collect();
                let issues: Vec<Text> = [false, true]
                    .into_iter()
                    .zip(issues)
                    .filter_map(|(old, issue)| recover_info(&dir, old, issue.as_ref()))
                    .flat_map(|info| atlas.font.split_lines(&info, w - 50.0))
                    .map(|(line, _)| line)
                    .collect();
                self.message_lines.set(message.len() + issues.len());
                let row = recover_buttons_top(h, message.len() + issues.len());
                let top = row - 8.0 - 8.0 - (message.len() + issues.len()) as f32 * 9.0 - 8.0 - 9.0;
                let center =
                    |text: &Text| (w / 2.0).floor() - (atlas.font.width_text(text) / 2.0).floor();
                batch.text_styled(atlas, &title, center(&title), top, u32::MAX, true);
                let mut y = top + 17.0;
                for line in &message {
                    batch.text_styled(atlas, line, center(line), y, u32::MAX, true);
                    y += 9.0;
                }
                // The issues widget is left aligned, the widget centred.
                let width = issues
                    .iter()
                    .map(|l| atlas.font.width_text(l))
                    .fold(0.0, f32::max);
                let x = (w / 2.0).floor() - (width / 2.0).floor();
                for line in &issues {
                    batch.text_styled(atlas, line, x, y + 8.0, u32::MAX, true);
                    y += 9.0;
                }
            }
            Mode::Recovered { success, .. } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                let title = lang::get_or("recover_world.done.title", "Recovery done");
                let message = if *success {
                    lang::get_or("recover_world.done.success", "Recovery was successful!")
                } else {
                    lang::get_or(
                        "recover_world.done.failed",
                        "Failed to recover from previous state.",
                    )
                };
                let lines = wrapped(atlas, &message, w - 50.0);
                self.message_lines.set(lines.len());
                // ConfirmScreen centres its column; AlertScreen puts the title at 70, the text at 90.
                let (top, text_top) = if *success {
                    let top = confirm_top(h, lines.len());
                    (top, top + 17.0)
                } else {
                    (70.0, 90.0)
                };
                centered(atlas, batch, &title, top, u32::MAX);
                for (i, line) in lines.iter().enumerate() {
                    centered(atlas, batch, line, text_top + i as f32 * 9.0, u32::MAX);
                }
            }
            Mode::BackupBeforeUpgrade {
                downgrade, stem, ..
            } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                // BackupStatus.DOWNGRADE is severe: its question is red (-2142128).
                let (question, warning, color) = match (downgrade, stem) {
                    (None, Some(StemIssue::Customized)) => (
                        lang::get_or(
                            "selectWorld.backupQuestion.customized",
                            "Customized worlds are no longer supported",
                        ),
                        lang::get_or(
                            "selectWorld.backupWarning.customized",
                            "Unfortunately, we do not support customized worlds in this version of Minecraft. We can still load this world and keep everything the way it was, but any newly generated terrain will no longer be customized. We're sorry for the inconvenience!",
                        ),
                        u32::MAX,
                    ),
                    (None, Some(StemIssue::Experimental)) => (
                        lang::get_or(
                            "selectWorld.backupQuestion.experimental",
                            "Worlds using Experimental Settings are not supported",
                        ),
                        lang::get_or(
                            "selectWorld.backupWarning.experimental",
                            "This world uses experimental settings that could stop working at any time. We cannot guarantee it will load or work. Here be dragons!",
                        ),
                        u32::MAX,
                    ),
                    (Some(version), _) => (
                        lang::get_or(
                            "selectWorld.backupQuestion.downgrade",
                            "Downgrading a world is not supported",
                        ),
                        translate(
                            "selectWorld.backupWarning.downgrade",
                            "This world was last played in version %s; you are on version %s. Downgrading a world could cause corruption - we cannot guarantee that it will load or work. If you still want to continue, please make a backup.",
                            &[version, &crate::save::VERSION_NAME],
                        ),
                        rgba(223, 80, 80, 255),
                    ),
                    (None, None) => (
                        lang::get_or(
                            "selectWorld.backupQuestion.file_fixing_required",
                            "Create a backup before upgrading this world?",
                        ),
                        lang::get_or(
                            "selectWorld.backupWarning.file_fixing_required",
                            "This world needs to be upgraded before you can play it. We strongly suggest creating a backup before continuing in case you experience world corruption.",
                        ),
                        u32::MAX,
                    ),
                };
                let lines = wrapped(atlas, &warning, w - 50.0);
                self.message_lines.set(lines.len());
                // BackupConfirmScreen.extractRenderState: the title at 50, the text from 70.
                centered(atlas, batch, &question, 50.0, color);
                for (i, line) in lines.iter().enumerate() {
                    centered(atlas, batch, line, 70.0 + i as f32 * 9.0, u32::MAX);
                }
            }
            Mode::OptimizeConfirm { erase_cache, .. } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                let description = lang::get_or(
                    "optimizeWorld.confirm.description",
                    "This will attempt to optimize your world by making sure all data is stored in the most recent game format. This can take a very long time, depending on your world. Once done, your world may play faster but will no longer be compatible with older versions of the game. Are you sure you wish to proceed?",
                );
                let lines = wrapped(atlas, &description, w - 50.0);
                self.message_lines.set(lines.len());
                centered(
                    atlas,
                    batch,
                    &lang::get_or("optimizeWorld.confirm.title", "Optimize World"),
                    50.0,
                    u32::MAX,
                );
                for (i, line) in lines.iter().enumerate() {
                    centered(atlas, batch, line, 70.0 + i as f32 * 9.0, u32::MAX);
                }
                // The Checkbox: at `width / 2 - 155 + 80`, `76 + textSize`.
                let label = lang::get_or("selectWorld.backupEraseCache", "Erase Cached Data");
                let width = BOX + 4.0 + HudBatch::text_width(atlas, &label).ceil();
                let x = (w / 2.0).floor() - 155.0 + 80.0;
                let y = 76.0 + (lines.len() as f32 + 1.0) * 9.0;
                self.erase_check.set([x, y, width, BOX]);
                crate::widgets::target([x, y, width, BOX]);
                let hover = (x..x + width).contains(&cursor.0) && (y..y + BOX).contains(&cursor.1);
                let sprite = match (*erase_cache, hover) {
                    (true, true) => "gui/sprites/widget/checkbox_selected_highlighted",
                    (true, false) => "gui/sprites/widget/checkbox_selected",
                    (false, true) => "gui/sprites/widget/checkbox_highlighted",
                    (false, false) => "gui/sprites/widget/checkbox",
                };
                batch.sprite_scaled(atlas, sprite, x, y, BOX, BOX, u32::MAX);
                batch.text(
                    atlas,
                    &label,
                    x + BOX + 4.0,
                    y + 4.0,
                    rgba(224, 224, 224, 255),
                    true,
                );
            }
            Mode::Optimize { world } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                self.draw_optimize(atlas, batch, w, h, &world.name);
            }
            Mode::LowDiskSpace { .. } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                let message = lang::get_or(
                    "selectWorld.warning.lowDiskSpace.description",
                    "There is not much space left on your device.\nRunning out of disk space while in game can lead to your world being damaged.",
                );
                let lines = wrapped(atlas, &message, w - 50.0);
                self.message_lines.set(lines.len());
                let top = confirm_top(h, lines.len());
                // ChatFormatting.RED.
                centered(
                    atlas,
                    batch,
                    &lang::get_or(
                        "selectWorld.warning.lowDiskSpace.title",
                        "Warning! Low disk space!",
                    ),
                    top,
                    rgba(255, 85, 85, 255),
                );
                for (i, line) in lines.iter().enumerate() {
                    centered(atlas, batch, line, top + 17.0 + i as f32 * 9.0, u32::MAX);
                }
            }
            Mode::MissingBiomeRegions { missing, .. } => {
                self.draw_missing_biome_regions(atlas, batch, (w, h), missing, phases)
            }
            Mode::Backup { .. } => {
                self.backup_shown.set(true);
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                centered(
                    atlas,
                    batch,
                    &lang::get_or("selectWorld.waitingForBackup.title", "Creating Backup"),
                    80.0,
                    u32::MAX,
                );
                let dots = ["O o o", "o O o", "o o O", "o O o"]
                    [(self.started.elapsed().as_millis() / 300 % 4) as usize];
                centered(atlas, batch, dots, 120.0, rgba(128, 128, 128, 255));
                let message = lang::get_or(
                    "selectWorld.waitingForBackup.message",
                    "Depending on the size of your world, this may take a while. Please do not close the game or shut off your device.",
                );
                for (i, line) in wrapped(atlas, &message, 360.0).iter().enumerate() {
                    centered(
                        atlas,
                        batch,
                        line,
                        132.0 + i as f32 * 9.0,
                        rgba(170, 170, 170, 255),
                    );
                }
            }
            Mode::Error { message, alert, .. } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                let lines: Vec<String> = message
                    .split('\n')
                    .flat_map(|line| wrapped(atlas, line, w - 50.0))
                    .collect();
                self.message_lines.set(lines.len());
                // AlertScreen: the title at 70, the text at 90.
                let (top, text_top, key, fallback, color) = if let Some(alert) = alert {
                    let (key, fallback, color) = match alert {
                        Alert::Incompatible => (
                            "selectWorld.incompatible.title",
                            "Incompatible version",
                            rgba(255, 0, 0, 255),
                        ),
                        Alert::Recreate => (
                            "selectWorld.recreate.error.title",
                            "An error occurred!",
                            u32::MAX,
                        ),
                        Alert::Symlink => (
                            "symlink_warning.title.world",
                            "The world folder contains links",
                            u32::MAX,
                        ),
                    };
                    (70.0, 90.0, key, fallback, color)
                } else {
                    let top = confirm_top(h, lines.len());
                    (
                        top,
                        top + 17.0,
                        "selectWorld.access_failure",
                        "Failed to access world",
                        u32::MAX,
                    )
                };
                centered(atlas, batch, &lang::get_or(key, fallback), top, color);
                for (i, line) in lines.iter().enumerate() {
                    centered(atlas, batch, line, text_top + i as f32 * 9.0, u32::MAX);
                }
            }
            Mode::MissingMods { missing, .. } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                self.draw_missing_mods(atlas, batch, (w, h), missing)
            }
            Mode::MultiplayerWarning { check } => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                let message = lang::get_or(
                    "multiplayerWarning.message",
                    "Caution: Online play is offered by third-party servers that are not owned, operated, or supervised by Mojang Studios or Microsoft. During online play, you may be exposed to unmoderated chat messages or other types of user-generated content that may not be suitable for everyone.",
                );
                // The message sits in a text box `width - 100` wide with 4 px
                // inside, left aligned (FittingMultiLineTextWidget).
                let box_w = w - 100.0;
                let lines = wrapped(atlas, &message, box_w - 8.0);
                self.message_lines.set(lines.len());
                let top = warning_top(h, lines.len());
                let header = Text::translate("multiplayerWarning.header", &[]).bold();
                batch.text_styled(
                    atlas,
                    &header,
                    (w / 2.0).floor() - (atlas.font.width_text(&header) / 2.0).floor(),
                    top,
                    u32::MAX,
                    true,
                );
                let box_x = ((w - box_w) / 2.0).floor();
                let box_y = top + 29.0;
                let box_h = lines.len() as f32 * 9.0 + 8.0;
                // WarningScreen's FittingMultiLineTextWidget narrates the
                // message; the title's StringWidget isn't active.
                crate::widgets::narratable(|| {
                    crate::widgets::Narratable::text(message.clone(), [box_x, box_y, box_w, box_h])
                });
                crate::ui::nine_slice(
                    batch,
                    atlas,
                    "gui/sprites/widget/text_field",
                    box_x,
                    box_y,
                    box_w,
                    box_h,
                    u32::MAX,
                );
                for (i, line) in lines.iter().enumerate() {
                    batch.text(
                        atlas,
                        line,
                        box_x + 4.0,
                        box_y + 4.0 + i as f32 * 9.0,
                        u32::MAX,
                        true,
                    );
                }
                // Checkbox: the box, its label right of it (as the dialogs draw theirs).
                let label =
                    lang::get_or("multiplayerWarning.check", "Do not show this screen again");
                let width = BOX + 4.0 + HudBatch::text_width(atlas, &label).ceil();
                let x = (w / 2.0).floor() - (width / 2.0).floor();
                let y = top + 57.0 + lines.len() as f32 * 9.0;
                self.warning_check.set([x, y, width, BOX]);
                crate::widgets::target([x, y, width, BOX]);
                let hover = (x..x + width).contains(&cursor.0) && (y..y + BOX).contains(&cursor.1);
                crate::widgets::narratable(|| {
                    crate::widgets::Narratable::checkbox(
                        label.clone(),
                        *check,
                        [x, y, width, BOX],
                        true,
                    )
                    .hovered(hover)
                });
                let sprite = match (*check, hover) {
                    (true, true) => "gui/sprites/widget/checkbox_selected_highlighted",
                    (true, false) => "gui/sprites/widget/checkbox_selected",
                    (false, true) => "gui/sprites/widget/checkbox_highlighted",
                    (false, false) => "gui/sprites/widget/checkbox",
                };
                batch.sprite_scaled(atlas, sprite, x, y, BOX, BOX, u32::MAX);
                batch.text(
                    atlas,
                    &label,
                    x + BOX + 4.0,
                    y + 4.0,
                    rgba(224, 224, 224, 255),
                    true,
                );
            }
            Mode::Onboarding(_) => {
                fade_from = Some(self.draw_onboarding(atlas, batch, w, h, phases))
            }
            Mode::BanNotice(notice) => {
                crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
                phases(crate::screen_draw::RenderPhase::Background, batch);
                let now = chrono::Utc::now().timestamp_millis();
                let layout = crate::confirm_screen::Choice::new(
                    &atlas.font,
                    (w, h),
                    &notice.title(),
                    &notice.message(now),
                    3,
                    100.0,
                );
                for (line, x, y) in &layout.lines {
                    batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
                }
                self.ban_buttons
                    .set([layout.buttons[0], layout.buttons[1], layout.buttons[2]]);
            }
            Mode::Multiplayer(screen) => screen.draw(
                atlas,
                batch,
                &mut self.caret.borrow_mut(),
                self.started,
                cursor,
                hide_server_address,
                phases,
            ),
            Mode::Realms(screen) => screen.draw(
                batch,
                atlas,
                &mut self.caret.borrow_mut(),
                self.started,
                cursor,
                w,
                h,
                self.focused_button.is_none(),
                phases,
            ),
        }
        // The hovered button's `Tooltip`, drawn once the widgets are.
        let mut button_tooltip = None;
        for b in self.buttons(w, h) {
            crate::widgets::record(&b.label, [b.x, b.y, b.w, 20.0], b.enabled);
            // Realms' icon buttons have no label to record.
            if b.enabled && b.label.trim().is_empty() {
                crate::widgets::target([b.x, b.y, b.w, 20.0]);
            }
            if let Some(action) = b.action.filter(|_| !self.preparing) {
                crate::widgets::declare_action(action, [b.x, b.y, b.w, 20.0], b.enabled);
            }
            let inside =
                cursor.0 >= b.x && cursor.0 < b.x + b.w && cursor.1 >= b.y && cursor.1 < b.y + 20.0;
            crate::widgets::narratable(|| self.button_narration(&b).hovered(inside));
            crate::widgets::widget_cursor(inside, b.enabled);
            if inside && let Some(key) = self.button_tooltip(b.id) {
                button_tooltip = Some((key, [b.x, b.y, b.w, 20.0]));
            }
            let is_focused = self.focused_button == Some(b.id);
            let hover = b.enabled && (inside || is_focused);
            let sprite = if !b.enabled {
                "gui/sprites/widget/button_disabled"
            } else if hover {
                "gui/sprites/widget/button_highlighted"
            } else {
                "gui/sprites/widget/button"
            };
            // Nine-slice approximation: left and right halves of the 200-wide sprite.
            if let Some(r) = atlas.sprites.get(sprite).copied() {
                let half = b.w / 2.0;
                let mut left = r;
                left.u1 = r.u0 + (r.u1 - r.u0) * (half / r.w as f32);
                batch.rect_uv(b.x, b.y, half, 20.0, left, u32::MAX);
                let mut right = r;
                right.u0 = r.u1 - (r.u1 - r.u0) * (half / r.w as f32);
                batch.rect_uv(b.x + half, b.y, half, 20.0, right, u32::MAX);
            }
            let color = if b.enabled {
                u32::MAX
            } else {
                rgba(160, 160, 160, 255)
            };
            if let Some((icon, size)) = b.icon {
                // CenteredIcon divides the button and sprite sizes separately:
                // a 15px icon in a 20px button starts at 10 - 7 = 3, not 2.
                batch.sprite_scaled(
                    atlas,
                    &format!("gui/sprites/{icon}"),
                    b.x + (b.w / 2.0).floor() - (size / 2.0).floor(),
                    b.y + 10.0 - (size / 2.0).floor(),
                    size,
                    size,
                    u32::MAX,
                );
                continue;
            }
            b.draw_label(batch, atlas, color);
            if let Some((icon, size)) = b.trailing {
                // TextAndIcon: `x + width - spriteWidth - 2`, centred in the height.
                batch.sprite_scaled(
                    atlas,
                    &format!("gui/sprites/{icon}"),
                    b.x + b.w - size - 2.0,
                    b.y + 10.0 - (size / 2.0).floor(),
                    size,
                    size,
                    u32::MAX,
                );
            }
        }
        // RealmsHeader's notification badges sit on their buttons, so they draw after.
        if let Mode::Realms(screen) = &self.mode {
            screen.draw_header_badges(atlas, batch, w);
        }
        if let Some(from) = fade_from {
            self.draw_onboarding_logo(atlas, batch, w, from);
        }
        phases(crate::screen_draw::RenderPhase::Foreground, batch);
        if let Mode::Create {
            name,
            creative,
            options,
            ..
        } = &self.mode
        {
            self.create_tooltip(atlas, batch, cursor, name, *creative, options);
        }
        if let Mode::Select { worlds, selected } = &self.mode {
            self.select_tooltip(atlas, batch, cursor, (w, h), worlds, *selected);
        }
        if let Mode::Multiplayer(screen) = &self.mode {
            screen.draw_tooltip(atlas, batch, cursor);
        }
        if let Mode::Realms(screen) = &self.mode {
            screen.draw_tooltip(atlas, batch, cursor, (w, h));
        }
        // WidgetTooltipHolder.refreshTooltipForNextRenderPass: beside the
        // pointer and kept by its button (`MenuTooltipPositioner`), split at
        // `Tooltip`'s 170. It comes after the onboarding fade: vanilla fades
        // the widgets, not the tooltip.
        if let Some((key, rect)) = button_tooltip {
            let lines = atlas.font.split(&Text::translate(key, &[]), 170.0);
            crate::ui::tooltip_menu(batch, atlas, &lines, cursor, rect);
        }
        if let Some(toast) = &self.toast {
            toast.draw(atlas, batch, 0.0);
        }
    }
}
