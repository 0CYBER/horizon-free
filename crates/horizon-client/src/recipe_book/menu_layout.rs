//! Load-boundary agreement between menu storage and recipe-book destinations.
use super::slot_layout::{CheckedSlotLayout, LayoutError};
use mod_api::RecipeBookLayout;

/// Content-owned menu metadata, independent of a mod's layout declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, mod_api::Wire)]
pub struct MenuLayoutMetadata {
    pub slots: u16,
    /// Contiguous square grid: first input, side length and computed result.
    pub crafting: Option<(u16, u16, u16)>,
}

pub fn compile(
    layout: RecipeBookLayout,
    menu: MenuLayoutMetadata,
) -> Result<CheckedSlotLayout, LayoutError> {
    let checked = CheckedSlotLayout::new(layout.clone(), menu.slots)?;
    if let Some((grid, size, output)) = menu.crafting {
        let RecipeBookLayout::Crafting {
            width,
            height,
            slots,
            result,
        } = &layout
        else {
            return Err(LayoutError::IncompatibleDisplay);
        };
        if *width != size
            || *height != size
            || *result != output
            || !slots
                .iter()
                .copied()
                .map(u32::from)
                .eq(u32::from(grid)..u32::from(grid) + u32::from(size) * u32::from(size))
        {
            return Err(LayoutError::InvalidGrid);
        }
    }
    Ok(checked.with_computed_result(menu.crafting.map(|(_, _, result)| result)))
}
