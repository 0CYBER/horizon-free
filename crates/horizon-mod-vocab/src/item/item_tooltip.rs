//! Moved here from the game so mods can name it.
use crate::ItemStack;
#[allow(unused_imports)]
use crate::*;
use horizon_core::nbt::Tag;
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// The `minecraft:tooltip_display` component applied by `ItemStack` while it
/// builds a hover tooltip.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct TooltipDisplay<'a> {
    pub(crate) component: Option<&'a Tag>,
    pub(crate) ominous_banner: bool,
}
impl<'a> TooltipDisplay<'a> {
    // hidden: a constructor for generated wire code, not for mods
    /// Reconstruct the read-only projection transported by the mod boundary.
    #[doc(hidden)]
    pub fn __wire_from_fields(component: Option<&'a Tag>, ominous_banner: bool) -> Self {
        Self {
            component,
            ominous_banner,
        }
    }

    /// Read tooltip visibility from an owned API stack's effective component patch.
    /// The game supplies this stack only after the tooltip subscription guard.
    pub fn for_stack(stack: &'a ItemStack) -> Self {
        Self::from_component(
            stack.components().and_then(|components| {
                components
                    .as_nbt()
                    .iter()
                    .find(|(id, _)| {
                        id.0.iter()
                            .copied()
                            .eq("minecraft:tooltip_display".encode_utf16())
                    })
                    .map(|(_, tag)| tag)
            }),
            false,
        )
    }

    /// Borrow a tooltip component without copying it. `ominous_banner` represents
    /// the vanilla banner default when the engine stores that default implicitly.
    pub fn from_component(component: Option<&'a Tag>, ominous_banner: bool) -> Self {
        Self {
            component,
            ominous_banner,
        }
    }

    fn hides_tooltip(&self) -> bool {
        matches!(
            self.component.and_then(|display| display.get("hide_tooltip")),
            Some(Tag::Byte(value)) if *value != 0
        )
    }

    /// `ItemStack.getTooltipLines`: `hide_tooltip` suppresses the name unless
    /// the tooltip flag is creative.
    pub fn name_visible(&self, creative: bool) -> bool {
        creative || !self.hides_tooltip()
    }

    /// `TooltipDisplay.shows`: whether details for a data component are shown.
    pub fn shows(&self, component: &str) -> bool {
        if self.hides_tooltip() {
            return false;
        }
        // `item_data::decode_components` models the raid captain banner's
        // default hidden pattern tooltip as `ominous_banner` instead of
        // retaining its redundant `tooltip_display` component.
        if self.ominous_banner
            && self.component.is_none()
            && component == "minecraft:banner_patterns"
        {
            return false;
        }
        let Some(Tag::List(hidden)) = self
            .component
            .and_then(|display| display.get("hidden_components"))
        else {
            return true;
        };
        !hidden.iter().any(|tag| {
            matches!(tag, Tag::String(id) if id.0.iter().copied().eq(component.encode_utf16()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_stack_tooltip_visibility_uses_its_patch() {
        let mut stack = ItemStack::of(crate::ItemKey(0));
        assert!(TooltipDisplay::for_stack(&stack).name_visible(false));
        stack
            .set_components(
                [(
                    "minecraft:tooltip_display".into(),
                    Tag::compound(vec![("hide_tooltip", Tag::Byte(1))]),
                )]
                .into(),
            )
            .unwrap();
        let display = TooltipDisplay::for_stack(&stack);
        assert!(!display.name_visible(false));
        assert!(display.name_visible(true));
        assert!(!display.shows("minecraft:lore"));
        stack
            .set_components(
                [(
                    "minecraft:tooltip_display".into(),
                    Tag::compound(vec![(
                        "hidden_components",
                        Tag::List(vec![Tag::String("minecraft:lore".into())]),
                    )]),
                )]
                .into(),
            )
            .unwrap();
        let display = TooltipDisplay::for_stack(&stack);
        assert!(display.name_visible(false));
        assert!(!display.shows("minecraft:lore"));
        assert!(display.shows("minecraft:attribute_modifiers"));
    }
}
