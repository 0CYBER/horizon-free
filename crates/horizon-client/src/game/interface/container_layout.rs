//! Container geometry constructed by the screen build seam.
use super::screen_build::ContainerLayout;
use super::{Creative, Game, Screen};
use horizon_client::inventory_effects::EffectsLayout;

#[cfg(test)]
thread_local! {
    pub(in crate::game) static LAYOUT_CONSTRUCTIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Selectors read by slot_position; item contents and scroll do not move slots.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct SlotInputs {
    count: usize,
    craft_size: usize,
    creative_inventory: bool,
    creative_tab: bool,
    station: Option<(std::mem::Discriminant<super::stations::Kind>, usize)>,
    container: Option<std::mem::Discriminant<horizon_client::containers::BlockEntity>>,
    machine: Option<bool>,
    crafter: bool,
    mount: Option<(usize, bool, bool)>,
    rows: Option<usize>,
    native: Option<(u64, Option<f32>, Option<f32>)>,
}

impl Game<'_> {
    fn native_slot_inputs(&self) -> Option<(u64, Option<f32>, Option<f32>)> {
        self.shown_native_screen().map(|s| {
            (
                s.layout_revision,
                self.native_screen_inventory_x(),
                self.native_screen_inventory_y(),
            )
        })
    }

    pub(super) fn container_layout(
        &self,
        batch: &horizon_client::hud::HudBatch,
    ) -> ContainerLayout {
        self.build_container_layout(batch, self.container_layout_inputs(batch))
    }

    pub(super) fn build_container_layout(
        &self,
        batch: &horizon_client::hud::HudBatch,
        mut inputs: ContainerLayout,
    ) -> ContainerLayout {
        #[cfg(test)]
        LAYOUT_CONSTRUCTIONS.with(|count| count.set(count.get() + 1));
        let origin = inputs.origin;
        let slots = self
            .slots()
            .into_iter()
            .map(|slot| {
                let (x, y) = self.slot_position(slot);
                (slot, [origin.0 + x - 1.0, origin.1 + y - 1.0, 18.0, 18.0])
            })
            .collect();
        inputs.effects = inputs
            .shows_effects
            .then(|| {
                self.inventory_effects_rect(
                    batch,
                    [inputs.origin.0, inputs.origin.1],
                    inputs.image[2],
                    &inputs.effects_layout,
                )
            })
            .flatten();
        inputs.slots = slots;
        inputs
    }

    /// The open container screen's layout from the game's state: the panel it
    /// draws, the book and effect panels beside it, the creative tab rows and
    /// every slot's hover box. The build calls it when it makes the layout;
    /// nothing else does.
    pub(super) fn container_layout_inputs(
        &self,
        batch: &horizon_client::hud::HudBatch,
    ) -> ContainerLayout {
        let image = self.container_image_parts(batch);
        let origin = self.inventory_origin_unbuilt(batch);
        // A title over the inventory draws no book or effects of its own.
        let covered = self.screen == Screen::Inventory && self.title.is_some();
        let book = (!covered).then(|| self.recipe_book_rect(batch)).flatten();
        let shows_effects = !covered
            && match self.screen {
                Screen::Creative => true,
                Screen::Inventory => {
                    self.container.is_none() && self.craft_size == 2 && !self.mount_active()
                }
                _ => false,
            };
        let space = Self::effects_space(batch, [origin.0, origin.1], image[2]);
        let effects_layout = if shows_effects {
            self.inventory_effects_layout(&space, image)
        } else {
            EffectsLayout::vanilla(space.available())
        };
        let tabs = (self.screen == Screen::Creative).then(|| {
            [
                [
                    image[0],
                    image[1] - Creative::TAB_ROW_DEPTH,
                    image[2],
                    Creative::TAB_ROW_DEPTH,
                ],
                [
                    image[0],
                    image[1] + image[3],
                    image[2],
                    Creative::TAB_ROW_DEPTH,
                ],
            ]
        });
        ContainerLayout {
            slot_inputs: SlotInputs {
                count: self.slot_count(),
                craft_size: self.craft_size,
                creative_inventory: self.creative_inventory_open(),
                creative_tab: self.creative_tab_open(),
                station: self
                    .station
                    .as_ref()
                    .map(|s| (std::mem::discriminant(&s.kind), s.inputs.len())),
                container: self.container_kind().map(std::mem::discriminant),
                machine: self.machine_layout_selector(),
                crafter: self.crafter_open(),
                mount: self.mount_layout_inputs(),
                rows: self.chest_rows(),
                native: self.native_slot_inputs(),
            },
            image,
            origin,
            book,
            effects: None,
            shows_effects,
            effects_revision: {
                use std::hash::{Hash, Hasher};
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                for effect in &self.potions.effects.list {
                    effect.id.hash(&mut hash);
                    effect.duration.hash(&mut hash);
                    effect.amplifier.hash(&mut hash);
                }
                hash.finish()
            },
            effects_layout,
            tabs,
            slots: Vec::new(),
        }
    }
}
