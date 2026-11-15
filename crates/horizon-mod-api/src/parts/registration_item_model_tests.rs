#![cfg(test)]
// Item model callbacks carry owned values and a separate runtime selection.
use crate::*;

struct NoClient;
impl Host for NoClient {}
crate::impl_host_areas!(NoClient);
impl ClientHost for NoClient {}
crate::impl_client_host_areas!(NoClient);

struct Selected;
#[crate::__subscriptions]
impl Hooks for Selected {
    fn item_model_property(&self, _: &dyn ClientHost, event: &mut ItemModelPropertyEvent<'_>) {
        event.value = event.stack.count() == 2 && event.bundle_selected == 3;
    }
    fn item_model_range_property(
        &self,
        _: &dyn ClientHost,
        event: &mut ItemModelRangePropertyEvent<'_>,
    ) {
        event.value = event.bundle_selected as f32 / event.stack.count() as f32;
    }
    fn item_model_select_property(
        &self,
        _: &dyn ClientHost,
        event: &mut ItemModelSelectPropertyEvent<'_>,
    ) {
        *event.value = (event.bundle_selected >= 0)
            .then(|| format!("{}:{}", event.bundle_selected, event.stack.count()));
    }
    fn item_tint_source(&self, _: &dyn ClientHost, event: &mut ItemTintSourceEvent<'_>) {
        event.color = if event.bundle_selected < 0 {
            u32::MAX
        } else {
            0xff00_0000 | ((event.bundle_selected as u32) << 16) | event.stack.count()
        };
    }
}

#[test]
fn model_callbacks_receive_owned_counts_and_the_exact_runtime_selection() {
    install_parts(&[], &crate::__private::part_tests::rule_schema(), |_| false).unwrap();
    let stack = crate::__private::part_tests::stack("minecraft:stick", 2, 0);
    let mods = vec![ModEntry::new("selected", Box::new(Selected))].into();
    testing::with_mods(mods, || {
        for (selected, boolean, number, text, tint) in [
            (3, true, 1.5, Some("3:2"), 0xff03_0002),
            (-1, false, -0.5, None, u32::MAX),
        ] {
            let mut value = false;
            dispatch_item_model_property(
                &NoClient,
                "example:selected",
                &stack,
                selected,
                &mut value,
            );
            assert_eq!(value, boolean);
            let mut value = 0.0;
            dispatch_item_model_range_property(
                &NoClient,
                "example:selected",
                &stack,
                selected,
                &mut value,
            );
            assert_eq!(value, number);
            let mut value = None;
            dispatch_item_model_select_property(
                &NoClient,
                "example:selected",
                &stack,
                selected,
                &mut value,
            );
            assert_eq!(value.as_deref(), text);
            let mut color = 0;
            dispatch_item_tint_source(
                &NoClient,
                "example:selected",
                &crate::Tag::compound([] as [(&str, crate::Tag); 0]),
                &stack,
                selected,
                &mut color,
            );
            assert_eq!(color, tint);
        }
    });
}

#[cfg(test)]
use super::*;

#[cfg(test)]
use crate::testing;
