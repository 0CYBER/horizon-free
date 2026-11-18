//! The question before a world opens that was saved with mods that are not loaded now (arismoko/horizon#3856):
//! opening it removes their blocks, and what those blocks hold, for good. Don't Open is the
//! default (and Escape); Back Up and Open opens only once the backup exists; Open Without a
//! Backup opens at once. The game asks (`Game::open_world`), since a `mod_mismatch` listener may
//! resolve a mod first; this screen only shows what is left and records the choice.
use super::*;
use crate::save::level_mods::{self, MissingMod};

#[cfg(test)]
#[path = "missing_mods_tests.rs"]
mod tests;

/// The same player-facing entry for visible text and narration.
fn entry(m: &MissingMod) -> String {
    let label = if m.saved_version.is_empty() {
        lang::tr("horizon.missing_mods.entry_unknown", &[&m.id])
    } else {
        lang::tr("horizon.missing_mods.entry", &[&m.id, &m.saved_version])
    };
    let kinds = level_mods::affected_kinds(m);
    if kinds.is_empty() {
        label
    } else {
        lang::tr("horizon.missing_mods.entry_kinds", &[&label, &kinds])
    }
}

/// The prompt's text: what opening does, then each missing mod.
fn message(missing: &[MissingMod]) -> String {
    let mut message = lang::get("horizon.missing_mods.description");
    for m in missing {
        message.push('\n');
        message.push_str(&entry(m));
    }
    message
}

/// What the narrator reads for the prompt.
pub(super) fn screen_info(missing: &[MissingMod]) -> crate::widgets::ScreenInfo {
    crate::widgets::ScreenInfo::new(
        "missing_mods",
        crate::widgets::join_for_narration(&[
            &lang::tr("horizon.missing_mods.count", &[&missing.len()]),
            &message(missing),
        ]),
    )
}

/// Shared viewport geometry for rendering, scrolling and action layout.
pub(super) fn layout(w: f32, h: f32) -> (f32, f32, bool) {
    let narrow = w < 324.0;
    let top = 36.0;
    let bottom = top + ((h - if narrow { 88.0 } else { 64.0 } - top) / 9.0).floor() * 9.0;
    (top, bottom, narrow)
}

impl Title {
    /// The right stick's scroll: the open list moves wherever the hidden pointer
    /// is, and Create World's buttons don't take the step.
    pub fn stick_scroll(&mut self, delta: f32, cursor: (f32, f32), size: (f32, f32)) {
        self.step_scroll(delta, crate::widgets::over_scroll_area(cursor), size);
    }
    /// A focus step past a list's edge: what the pointer is over scrolls, and
    /// Create World's buttons don't take the step.
    pub fn step_scroll(&mut self, delta: f32, cursor: (f32, f32), size: (f32, f32)) {
        let open_rules = self.rules.get_mut().is_some();
        if matches!(self.mode, Mode::Create { .. }) && !open_rules {
            return;
        }
        self.scroll_worlds(delta, cursor, size);
    }
    /// The scrolling area of the bounded message the open screen shows (`top`, `bottom`): a missing-mods
    /// prompt's list, or a world request's question. None on any other screen.
    fn message_bounds(&self, (w, h): (f32, f32)) -> Option<(f32, f32)> {
        match self.mode {
            Mode::MissingMods { .. } => {
                let (top, bottom, _) = layout(w, h);
                Some((top, bottom))
            }
            Mode::WorldRequest { .. } => {
                let area = crate::title::world_request_layout((w, h));
                Some((area.top, area.bottom))
            }
            _ => None,
        }
    }
    pub(super) fn drag_missing_mods(&mut self, cursor: (f32, f32), size: (f32, f32)) -> bool {
        let Some((top, bottom)) = self.message_bounds(size) else {
            return false;
        };
        let mut scroll = self.scroll.get();
        scroll.drag(cursor.1, top, bottom, self.message_lines.get() as f32 * 9.0);
        self.scroll.set(scroll);
        true
    }
    pub(super) fn wheel_missing_mods(&mut self, delta: f32, size: (f32, f32)) -> bool {
        let Some((top, bottom)) = self.message_bounds(size) else {
            return false;
        };
        let mut scroll = self.scroll.get();
        scroll.wheel(
            delta,
            18.0,
            top,
            bottom,
            self.message_lines.get() as f32 * 9.0,
        );
        self.scroll.set(scroll);
        true
    }
    pub(super) fn press_missing_mods(&mut self, cursor: (f32, f32), size: (f32, f32)) -> Action {
        let Some((top, bottom)) = self.message_bounds(size) else {
            return Action::None;
        };
        self.scroll.get_mut().press(
            cursor,
            size.0 - 10.0,
            top,
            bottom,
            self.message_lines.get() as f32 * 9.0,
        );
        Action::None
    }
    pub(super) fn confirm_missing_mods(&mut self) -> Option<(Action, bool)> {
        let Mode::MissingMods { world, .. } = &self.mode else {
            return None;
        };
        Some((
            self.answer_missing_mods(world.clone(), self.focused_button.unwrap_or(101)),
            true,
        ))
    }

    /// The controller draws its own focus mark, so remove the keyboard's mark
    /// when control passes to the existing pad focus path (including its first stop).
    pub fn missing_mods_pad_focus(&mut self) {
        if matches!(self.mode, Mode::MissingMods { .. }) {
            self.focused_button = None;
        }
    }

    /// Page keys scroll the complete warning (or a world request's question) regardless of which action has
    /// focus. Arrow keys remain available for ordinary action navigation.
    pub fn page_missing_mods(&mut self, key: winit::keyboard::KeyCode, (w, h): (f32, f32)) -> bool {
        use winit::keyboard::KeyCode;
        let Some((top, bottom)) = self.message_bounds((w, h)) else {
            return false;
        };
        let content = self.message_lines.get() as f32 * 9.0;
        let mut scroll = self.scroll.get();
        let page = (bottom - top).max(9.0);
        scroll.amount = match key {
            KeyCode::PageUp => scroll.amount - page,
            KeyCode::PageDown => scroll.amount + page,
            KeyCode::Home => 0.0,
            KeyCode::End => content,
            _ => return false,
        };
        scroll.clamp(top, bottom, content);
        self.scroll.set(scroll);
        true
    }

    /// Shows the question for the world in `folder`, whose `missing` mods no listener resolved.
    pub fn ask_missing_mods(&mut self, folder: &str, missing: Vec<MissingMod>) {
        let world = list_worlds(&self.saves)
            .into_iter()
            .find(|w| w.folder == folder)
            .unwrap_or_else(|| {
                WorldEntry::corrupted(folder.to_owned(), std::time::SystemTime::now())
            });
        self.set_mode(Mode::MissingMods { world, missing });
        self.focused_button = Some(101);
        self.scroll.set(Default::default());
    }

    /// The prompt's contents; Title draws the menu background before its background hook.
    pub(super) fn draw_missing_mods(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        (w, h): (f32, f32),
        missing: &[MissingMod],
    ) {
        let width = w - 32.0;
        let mut lines = wrapped(atlas, &lang::get("horizon.missing_mods.description"), width);
        let description_lines = lines.len();
        lines.push(String::new());
        let mut rows = Vec::with_capacity(missing.len());
        for m in missing {
            let label = entry(m);
            let start = lines.len();
            lines.extend(wrapped(atlas, &label, width));
            rows.push((label, start, lines.len() - start));
            lines.push(String::new());
        }
        self.message_lines.set(lines.len());
        let (top, bottom, _) = layout(w, h);
        let mut scroll = self.scroll.get();
        scroll.clamp(top, bottom, lines.len() as f32 * 9.0);
        self.scroll.set(scroll);
        centered(
            atlas,
            batch,
            &lang::tr("horizon.missing_mods.count", &[&missing.len()]),
            12.0,
            rgba(255, 85, 85, 255),
        );
        for (label, start, count) in rows {
            let y = top + start as f32 * 9.0 - (scroll.amount / 9.0).floor() * 9.0;
            super::flat::row_target(&label, [16.0, y, width, count as f32 * 9.0], top, bottom);
        }
        let first = batch.vertices.len();
        for (i, line) in lines.iter().enumerate() {
            centered(
                atlas,
                batch,
                line,
                top + i as f32 * 9.0 - (scroll.amount / 9.0).floor() * 9.0,
                if i < description_lines {
                    u32::MAX
                } else {
                    rgba(170, 170, 170, 255)
                },
            );
        }
        batch.clip_since(first, 0.0, top, w, bottom);
        crate::ui::scrollbar(
            batch,
            atlas,
            w - 10.0,
            top,
            bottom,
            lines.len() as f32 * 9.0,
            scroll.amount,
        );
    }

    /// The question's buttons: Don't Open (101), Back Up and Open (102), Open Without a Backup
    /// (103).
    pub(super) fn answer_missing_mods(&mut self, world: WorldEntry, id: u8) -> Action {
        match id {
            102 => self.back_up_for_missing_mods(world),
            103 => return self.open_with_missing_mods(world),
            _ => self.leave_closed_for_missing_mods(),
        }
        Action::None
    }

    /// Don't Open (and Escape): the world stays closed and as it was, and is asked about again
    /// next time.
    pub(super) fn leave_closed_for_missing_mods(&mut self) {
        level_mods::forget_answer();
        let folder = match &self.mode {
            Mode::MissingMods { world, .. } => Some(world.folder.clone()),
            _ => None,
        };
        match folder {
            Some(folder) => self.back_from_world(&folder),
            None => self.return_to_list(None),
        }
    }

    /// Back Up and Open: a full copy of the world first; it opens once the copy exists.
    fn back_up_for_missing_mods(&mut self, world: WorldEntry) {
        self.open_after_mods_backup = true;
        self.start_backup(world);
    }

    /// The backup Back Up and Open asked for is done: the world opens, with vanilla's "Backed up"
    /// toast naming where the backup is, or, when it could not be made, stays closed and says why.
    pub(super) fn mods_backup_done(
        &mut self,
        world: WorldEntry,
        result: std::result::Result<(PathBuf, u64), String>,
    ) {
        match result {
            Ok((path, bytes)) => {
                log::info!("Horizon: backed up {} to {}", world.folder, path.display());
                self.return_to_list(Some(&world.folder));
                let folder = path.display().to_string();
                self.pending_toast = Some(WorldToast::Created { folder, bytes });
                if let Action::Play { folder } = self.open_with_missing_mods(world) {
                    self.open_request = Some(folder);
                }
            }
            Err(why) => {
                level_mods::forget_answer();
                self.return_to_list(Some(&world.folder));
                self.failed(lang::tr("horizon.missing_mods.backup_failed", &[&why]));
            }
        }
    }

    /// Open Without a Backup, or the backup made: the player chose to open the world anyway.
    fn open_with_missing_mods(&mut self, world: WorldEntry) -> Action {
        level_mods::confirm(&self.saves.join(&world.folder));
        Action::Play {
            folder: world.folder,
        }
    }
}

impl Title {
    /// Polling is done once per rendered frame so filesystem work never blocks drawing.
    pub fn poll_backup(&mut self) {
        self.poll_realms();
        self.poll_onboarding(std::time::Instant::now());
        self.poll_account();
        self.poll_optimize();
        if !self.backup_shown.get() {
            return;
        }
        let Some(receiver) = &self.backup else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("Backup worker stopped".into()),
        };
        self.backup = None;
        if let Mode::Backup { world } = &self.mode {
            let world = world.clone();
            let folder = world.folder.clone();
            if std::mem::take(&mut self.open_after_mods_backup) {
                return self.mods_backup_done(world, result);
            }
            if std::mem::take(&mut self.restore_after_backup) {
                // `makeBackupAndShowToast(..).thenAcceptAsync(..)` in attemptRestore: the restore
                // runs, backup failed or not.
                self.attempt_restore(world.clone());
            } else if let Some(erase_cache) = self.optimize_after_backup.take() {
                // `conditionallyMakeBackupAndShowToast(..).thenAcceptAsync(..)`: the toast shows
                // and OptimizeWorldScreen opens, backup failed or not.
                self.begin_optimize(world.clone(), erase_cache);
            } else {
                self.return_to_list(Some(&folder));
            }
            // EditWorldScreen opens the world after the backup, failed or not: through the
            // checks WorldOpenFlows still has to make.
            if let Some(stem_asked) = self.backup_then_open.take()
                && let Action::Play { folder } = self.after_prompt(world.clone(), stem_asked)
            {
                self.open_request = Some(folder);
            }
            let toast = match result {
                Ok((_, bytes)) => WorldToast::Created { folder, bytes },
                Err(error) => WorldToast::Failed(error),
            };
            self.pending_toast = Some(toast);
        }
    }
}
