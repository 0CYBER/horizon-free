//! EditWorldScreen, and its Horizon opt-in (arismoko/horizon#2554): when the installed mods' biome regions
//! differ from the ones the world saved, the world's owner may adopt them for new chunks. The
//! choice is written into the world before it opens again; chunks that exist keep their biomes.
//! Design: `docs/design/mod-api/mod-biome-regions.md`.
use super::*;

#[cfg(test)]
#[path = "edit_world_tests.rs"]
mod tests;

/// What the narrator reads for the opt-in's question.
pub(super) fn adopt_screen_info() -> crate::widgets::ScreenInfo {
    crate::widgets::ScreenInfo::new(
        "adopt_biome_regions",
        crate::widgets::join_for_narration(&[
            &lang::get("horizon.biome_regions.adopt.title"),
            &lang::get("horizon.biome_regions.adopt.description"),
        ]),
    )
}

impl Title {
    /// Opens Edit World on `world` with `name` in its field; the opt-in is offered only while
    /// adopting would change the world's saved regions.
    pub(super) fn edit_world(&mut self, name: String, world: WorldEntry) {
        let adopt_regions = crate::save::biome_regions::adoptable(&self.saves.join(&world.folder));
        self.set_mode(Mode::Edit {
            world,
            name,
            focused: true,
            adopt_regions,
        });
    }

    pub(super) fn draw_edit(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        (w, h): (f32, f32),
        name: &str,
        focused: bool,
        phases: &mut dyn FnMut(crate::screen_draw::RenderPhase, &mut HudBatch),
    ) {
        crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        phases(crate::screen_draw::RenderPhase::Background, batch);
        centered(
            atlas,
            batch,
            &lang::get_or("selectWorld.edit.title", "Edit World"),
            15.0,
            u32::MAX,
        );
        let top = edit_top(h);
        batch.text(
            atlas,
            &lang::get_or("selectWorld.enterName", "World Name"),
            (w / 2.0).floor() - 100.0,
            top + 25.0,
            rgba(160, 160, 160, 255),
            true,
        );
        self.caret.borrow_mut().draw(
            batch,
            atlas,
            (EDIT_NAME, name, 32),
            [(w / 2.0).floor() - 100.0, top + 39.0, 200.0, 20.0],
            focused,
            self.started,
        );
        crate::widgets::name_last_edit_box(|| narration_text("selectWorld.enterName", &[]));
    }

    pub fn confirm_edit(&mut self) {
        let Mode::Edit {
            world,
            name,
            focused: true,
            ..
        } = &self.mode
        else {
            return;
        };
        if horizon_core::java_character::is_blank(name) {
            return;
        }
        let folder = world.folder.clone();
        match world_management::rename(&self.saves, &folder, name) {
            Ok(()) => self.return_to_list(Some(&folder)),
            Err(error) => self.failed(error),
        }
    }

    pub(super) fn draw_adopt_regions(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        (w, h): (f32, f32),
        phases: &mut dyn FnMut(crate::screen_draw::RenderPhase, &mut HudBatch),
    ) {
        crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        phases(crate::screen_draw::RenderPhase::Background, batch);
        let description = lang::get("horizon.biome_regions.adopt.description");
        let lines = wrapped(atlas, &description, w - 50.0);
        self.message_lines.set(lines.len());
        let top = confirm_top(h, lines.len());
        centered(
            atlas,
            batch,
            &lang::get("horizon.biome_regions.adopt.title"),
            top,
            u32::MAX,
        );
        for (i, line) in lines.iter().enumerate() {
            centered(atlas, batch, line, top + 17.0 + i as f32 * 9.0, u32::MAX);
        }
    }

    /// The opt-in's answer: `adopt` writes the installed regions into the world's record, synced
    /// to the disk, so its next chunk already uses them. Either way Edit World comes back with
    /// the name as it was typed.
    pub(super) fn answer_adopt_regions(
        &mut self,
        world: WorldEntry,
        name: String,
        adopt: bool,
    ) -> Action {
        if adopt
            && let Err(why) =
                crate::save::biome_regions::choose(&self.saves.join(&world.folder), false, true)
        {
            self.failed(lang::tr("horizon.biome_regions.save_failed", &[&why]));
            return Action::None;
        }
        self.edit_world(name, world);
        Action::None
    }
}
