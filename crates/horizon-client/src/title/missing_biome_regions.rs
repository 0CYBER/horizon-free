//! The question before a world opens whose saved biome regions name mods that are not
//! installed (arismoko/horizon#2554): open it with vanilla's biomes where those regions were, recorded in the
//! world, or do not open it.
use super::*;

#[cfg(test)]
#[path = "missing_biome_regions_tests.rs"]
pub(super) mod tests;

/// The prompt's text: what the choice does, then each region and its mod.
fn message(missing: &[(String, String)]) -> String {
    let mut message = lang::get("horizon.biome_regions.missing.description");
    for (owner, region) in missing {
        message.push('\n');
        message.push_str(&lang::tr(
            "horizon.biome_regions.missing.region",
            &[region, owner],
        ));
    }
    message
}

/// What the narrator reads for the prompt.
pub(super) fn screen_info(missing: &[(String, String)]) -> crate::widgets::ScreenInfo {
    crate::widgets::ScreenInfo::new(
        "missing_biome_regions",
        crate::widgets::join_for_narration(&[
            &lang::get("horizon.biome_regions.missing.title"),
            &message(missing),
        ]),
    )
}

impl Title {
    /// The world's regions without a recorded choice, if any: then the prompt is shown and
    /// the world waits for the answer.
    pub(super) fn ask_missing_biome_regions(&mut self, world: &WorldEntry) -> bool {
        let missing = crate::save::biome_regions::unchosen_missing(&self.saves.join(&world.folder));
        if missing.is_empty() {
            return false;
        }
        self.set_mode(Mode::MissingBiomeRegions {
            world: world.clone(),
            missing,
        });
        true
    }

    pub(super) fn draw_missing_biome_regions(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        (w, h): (f32, f32),
        missing: &[(String, String)],
        phases: &mut dyn FnMut(crate::screen_draw::RenderPhase, &mut HudBatch),
    ) {
        crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        phases(crate::screen_draw::RenderPhase::Background, batch);
        let lines = wrapped(atlas, &message(missing), w - 50.0);
        self.message_lines.set(lines.len());
        let top = confirm_top(h, lines.len());
        centered(
            atlas,
            batch,
            &lang::get("horizon.biome_regions.missing.title"),
            top,
            rgba(255, 85, 85, 255),
        );
        for (i, line) in lines.iter().enumerate() {
            centered(atlas, batch, line, top + 17.0 + i as f32 * 9.0, u32::MAX);
        }
    }

    /// "Open with vanilla biomes": the choice is in the world before it opens, so its first
    /// new chunk already uses it.
    pub(super) fn open_with_vanilla_regions(&mut self, world: WorldEntry) -> Action {
        let dir = self.saves.join(&world.folder);
        match crate::save::biome_regions::choose(&dir, true, false) {
            Ok(()) => self.check_disk_space(world),
            Err(why) => {
                self.failed(lang::tr("horizon.biome_regions.save_failed", &[&why]));
                Action::None
            }
        }
    }
}

impl Title {
    /// `openWorldCheckDiskSpace`: the last check before a world loads, after its prompts.
    pub(super) fn check_disk_space(&mut self, world: WorldEntry) -> Action {
        if crate::save::low_disk_space(&self.saves.join(&world.folder)) {
            self.set_mode(Mode::LowDiskSpace { world });
            return Action::None;
        }
        Action::Play {
            folder: world.folder,
        }
    }
}
