//! Checked, menu-order slot mapping shared by custom-book ghosts and placement.
//!
//! These roles are Horizon declarations: NeoForge 26.3 public documentation
//! names no registration counterpart for vanilla's client `GhostSlots`.
#[cfg(test)]
#[path = "slot_layout_tests.rs"]
mod tests;
use mod_api::{RecipeBookDisplay, RecipeBookLayout as SlotLayout, RecipeSlotDisplay};

/// A layout frozen after validating against a menu's container slot count.
#[derive(Clone, Debug)]
pub struct CheckedSlotLayout(SlotLayout, Option<u16>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    InvalidSlotCount,
    InvalidGrid,
    OutOfRange(u16),
    DuplicateSlot(u16),
    IncompatibleDisplay,
    InvalidDisplayGrid,
    MissingGeometry(u16),
}

impl CheckedSlotLayout {
    /// The frozen destination declaration, for compiling authoritative placement.
    pub fn declaration(&self) -> &SlotLayout {
        &self.0
    }
    /// Only an instantaneous crafting backend owns a computed, non-storage result.
    pub fn computed_result(&self) -> Option<u16> {
        self.1
    }
    pub(crate) fn with_computed_result(mut self, result: Option<u16>) -> Self {
        self.1 = result;
        self
    }
    /// Validate mod-controlled indices once before mapping any display.
    /// The menu limit is the existing 256 container slots, not a display cap.
    pub fn new(layout: SlotLayout, slot_count: u16) -> Result<Self, LayoutError> {
        if slot_count == 0 || slot_count > 256 {
            return Err(LayoutError::InvalidSlotCount);
        }
        if let SlotLayout::Crafting {
            width,
            height,
            slots,
            ..
        } = &layout
        {
            let cells = u32::from(*width) * u32::from(*height);
            if *width == 0
                || *height == 0
                || cells > u32::from(slot_count)
                || cells as usize != slots.len()
            {
                return Err(LayoutError::InvalidGrid);
            }
        }
        let checked = Self(layout, None);
        let mut seen = [false; 256];
        for slot in checked.slots() {
            if slot >= slot_count {
                return Err(LayoutError::OutOfRange(slot));
            }
            if seen[usize::from(slot)] {
                return Err(LayoutError::DuplicateSlot(slot));
            }
            seen[usize::from(slot)] = true;
        }
        Ok(checked)
    }

    fn slots(&self) -> Vec<u16> {
        match &self.0 {
            SlotLayout::Crafting { slots, result, .. } => {
                slots.iter().copied().chain([*result]).collect()
            }
            SlotLayout::Furnace {
                input,
                fuel,
                result,
            } => vec![*input, *fuel, *result],
            SlotLayout::Stonecutter { input, result } => vec![*input, *result],
            SlotLayout::Smithing {
                template,
                base,
                addition,
                result,
            } => vec![*template, *base, *addition, *result],
        }
    }

    /// All mapped slots must have finite typed screen geometry. Called when a
    /// screen is bound, never per frame; omitted slots cannot receive ghosts.
    pub fn validate_geometry(&self, geometry: &[Option<[f32; 2]>]) -> Result<(), LayoutError> {
        for slot in self.slots() {
            if !geometry
                .get(usize::from(slot))
                .and_then(|p| *p)
                .is_some_and(|p| p.into_iter().all(f32::is_finite))
            {
                return Err(LayoutError::MissingGeometry(slot));
            }
        }
        Ok(())
    }

    /// Map unresolved display alternatives without changing inventory. The
    /// existing bounded display resolver owns expansion and candidate budgets.
    /// `fuel_empty` is sampled from the authoritative synchronized fuel slot.
    pub fn map(
        &self,
        display: &RecipeBookDisplay,
        fuel_empty: bool,
    ) -> Result<Vec<(u16, RecipeSlotDisplay)>, LayoutError> {
        use RecipeBookDisplay as D;
        use SlotLayout as L;
        let mut out = Vec::new();
        match (&self.0, display) {
            (
                L::Crafting {
                    width,
                    height,
                    slots,
                    result,
                },
                D::Shaped {
                    width: w,
                    height: h,
                    ingredients,
                    result: output,
                    ..
                },
            ) => {
                if *w == 0
                    || *h == 0
                    || *w > u32::from(*width)
                    || *h > u32::from(*height)
                    || w.checked_mul(*h).map(|n| n as usize) != Some(ingredients.len())
                {
                    return Err(LayoutError::InvalidDisplayGrid);
                }
                for (ingredient, index) in super::place_cells(
                    usize::from(*width),
                    usize::from(*height),
                    *w as usize,
                    *h as usize,
                    ingredients,
                ) {
                    out.push((slots[index], ingredient));
                }
                out.push((*result, output.clone()));
            }
            (
                L::Crafting {
                    width,
                    height,
                    slots,
                    result,
                },
                D::Shapeless {
                    ingredients,
                    result: output,
                    ..
                },
            ) => {
                if ingredients.len() > slots.len() {
                    return Err(LayoutError::InvalidDisplayGrid);
                }
                for (ingredient, index) in super::place_cells(
                    usize::from(*width),
                    usize::from(*height),
                    usize::from(*width),
                    usize::from(*height),
                    ingredients,
                ) {
                    out.push((slots[index], ingredient));
                }
                out.push((*result, output.clone()));
            }
            (
                L::Furnace {
                    input,
                    fuel,
                    result,
                },
                D::Furnace {
                    ingredient,
                    fuel: fuel_display,
                    result: output,
                    ..
                },
            ) => {
                out.push((*input, ingredient.clone()));
                if fuel_empty {
                    out.push((*fuel, fuel_display.clone()));
                }
                out.push((*result, output.clone()));
            }
            (
                L::Stonecutter { input, result },
                D::Stonecutter {
                    input: ingredient,
                    result: output,
                    ..
                },
            ) => {
                out.push((*input, ingredient.clone()));
                out.push((*result, output.clone()));
            }
            (
                L::Smithing {
                    template,
                    base,
                    addition,
                    result,
                },
                D::Smithing {
                    template: t,
                    base: b,
                    addition: a,
                    result: output,
                    ..
                },
            ) => {
                out.extend([
                    (*template, t.clone()),
                    (*base, b.clone()),
                    (*addition, a.clone()),
                    (*result, output.clone()),
                ]);
            }
            _ => return Err(LayoutError::IncompatibleDisplay),
        }
        out.retain(|(_, display)| !matches!(display, RecipeSlotDisplay::Empty));
        Ok(out)
    }
}
