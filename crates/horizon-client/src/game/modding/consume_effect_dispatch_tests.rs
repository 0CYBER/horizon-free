//! Explicit living consume effects reach real native listeners, including nested kinds.
#![cfg(test)]
use crate::game::{api_values::TryIntoApi, native_entity_refs::tests::with_game};
use horizon_client::{
    items::{ConsumeEffect, ItemStack},
    mobs::Mob,
};
use horizon_core::{geometry::Vec3, nbt::Tag};
use mod_api::{ConsumeEffectEvent, Hooks, ModEntry, ServerHost};
use std::sync::{Arc, Mutex};

struct First(Arc<Mutex<Vec<&'static str>>>);
#[mod_api::__subscriptions]
impl Hooks for First {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        if event.kind != "first:effect" {
            return;
        }
        self.0.lock().unwrap().push("first");
        let source = host.entity(21).unwrap();
        let living = host.living(source).unwrap();
        let same = ItemStack::new("minecraft:pufferfish", 1)
            .try_into_api()
            .unwrap();
        assert_eq!(living.apply_consume_effect(host, &same, 0), Ok(Some(false)));
        let other = ItemStack::new("minecraft:honey_bottle", 1)
            .try_into_api()
            .unwrap();
        assert_eq!(living.apply_consume_effect(host, &other, 0), Ok(Some(true)));
        self.0.lock().unwrap().push("first_after");
    }
}
struct Second(Arc<Mutex<Vec<&'static str>>>);
#[mod_api::__subscriptions]
impl Hooks for Second {
    fn consume_effect(&self, _: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        if event.kind == "second:effect" {
            self.0.lock().unwrap().push("second");
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_living_handle_dispatches_custom_kinds_with_same_kind_recursion_refused() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![
        ModEntry::new("first", Box::new(First(trace.clone()))),
        ModEntry::new("second", Box::new(Second(trace.clone()))),
    ];
    mod_platform::testing::with_mods(mods.into(), || {
        with_game(|game| {
            let mut items = game.items().clone();
            for (item, kind) in [
                ("minecraft:pufferfish", "first:effect"),
                ("minecraft:honey_bottle", "second:effect"),
            ] {
                items
                    .items
                    .get_mut(item)
                    .unwrap()
                    .consumable
                    .as_mut()
                    .unwrap()
                    .effects = vec![ConsumeEffect::Native {
                    kind: kind.to_owned(),
                    data: Arc::new(Tag::Compound(Default::default())),
                }];
            }
            game.server_items = Some(Arc::new(items));
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs
                .push(Mob::new(21, kind, Vec3::new(0.0, 64.0, 0.0), 0.0));
            let living = game.living(game.entity(21).unwrap()).unwrap();
            let stack = ItemStack::new("minecraft:pufferfish", 1)
                .try_into_api()
                .unwrap();
            assert_eq!(living.apply_consume_effect(game, &stack, 0), Ok(Some(true)));
            assert_eq!(*trace.lock().unwrap(), ["first", "second", "first_after"]);
            assert_eq!(living.apply_consume_effect(game, &stack, 0), Ok(Some(true)));
            assert_eq!(
                *trace.lock().unwrap(),
                [
                    "first",
                    "second",
                    "first_after",
                    "first",
                    "second",
                    "first_after"
                ]
            );
        });
    });
}
