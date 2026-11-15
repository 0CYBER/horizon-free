//! Frozen authoritative requirements mapped to checked menu-owned destinations.
use crate::{crafting::Placement, recipe_book::slot_layout::CheckedSlotLayout};
use mod_api::RecipeBookLayout as L;

#[derive(Clone, Debug)]
pub struct MenuPlacement {
    pub(super) menu: String,
    pub(super) placement: Placement,
    pub(super) inputs: Vec<usize>,
    pub(super) cells: Vec<(usize, usize)>,
    pub(super) clear: Vec<usize>,
    pub(super) virtual_result: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementRefusal {
    NotPlaceable,
    InvalidLayout,
    Unavailable,
    Denied,
    NoRoom,
    Changed,
}

impl MenuPlacement {
    /// Compile once per table load. Display candidates are never consulted.
    pub fn new(
        menu: String,
        placement: Placement,
        layout: &CheckedSlotLayout,
    ) -> Result<Self, PlacementRefusal> {
        if placement.impossible() || placement.ingredients.is_empty() {
            return Err(PlacementRefusal::NotPlaceable);
        }
        let (width, height, inputs, result, virtual_result) = match layout.declaration() {
            L::Crafting {
                width,
                height,
                slots,
                result,
            } => (
                usize::from(*width),
                usize::from(*height),
                slots.iter().copied().map(usize::from).collect::<Vec<_>>(),
                usize::from(*result),
                layout.computed_result().map(usize::from),
            ),
            L::Furnace { input, result, .. } | L::Stonecutter { input, result } => {
                (1, 1, vec![usize::from(*input)], usize::from(*result), None)
            }
            L::Smithing {
                template,
                base,
                addition,
                result,
            } => (
                3,
                1,
                vec![
                    usize::from(*template),
                    usize::from(*base),
                    usize::from(*addition),
                ],
                usize::from(*result),
                None,
            ),
        };
        let (w, h) = placement.shaped.unwrap_or((width, height));
        if w == 0
            || h == 0
            || w > width
            || h > height
            || placement.slots.len() > inputs.len()
            || placement.ingredients.len() > inputs.len()
            || placement.shaped.is_some() && w.checked_mul(h) != Some(placement.slots.len())
            || !matches!(layout.declaration(), L::Crafting { .. })
                && (placement.shaped.is_some() || placement.slots.len() != inputs.len())
        {
            return Err(PlacementRefusal::InvalidLayout);
        }
        let mut seen = vec![false; placement.ingredients.len()];
        for index in &placement.slots {
            if *index == -1 {
                continue;
            }
            let index = usize::try_from(*index).map_err(|_| PlacementRefusal::InvalidLayout)?;
            let used = seen.get_mut(index).ok_or(PlacementRefusal::InvalidLayout)?;
            if *used {
                return Err(PlacementRefusal::InvalidLayout);
            }
            *used = true;
        }
        if seen.iter().any(|used| !*used) {
            return Err(PlacementRefusal::InvalidLayout);
        }
        let cells = crate::recipe_book::place_cells(width, height, w, h, &placement.slots)
            .into_iter()
            .filter(|(ingredient, _)| *ingredient != -1)
            .map(|(ingredient, cell)| (ingredient as usize, inputs[cell]))
            .collect();
        let mut clear = inputs.clone();
        if virtual_result.is_none() && !matches!(layout.declaration(), L::Crafting { .. }) {
            clear.push(result);
        }
        Ok(Self {
            menu,
            placement,
            inputs,
            cells,
            clear,
            virtual_result,
        })
    }
}
