//! `ItemPart::creator_mod_id` behind `ItemResource::creator_mod_id`'s read.
use super::native_entity_refs::tests::with_game;
use horizon_client::items::ItemStack;
use mod_api::{CreatorModIdEvent, Hooks, ItemPart, Registrar, RegistrationContext};

/// A bucket's Part: a stack of more than one stands for another mod's content.
#[derive(Clone)]
struct Bucket;

#[mod_api::__subscriptions]
impl Hooks for Bucket {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("minecraft:bucket");
        r.item_part(key, self.clone());
        let stick = r.item_key("minecraft:stick");
        r.item_part(stick, NoMod);
    }
}
#[mod_api::horizon_part]
impl ItemPart for Bucket {
    fn creator_mod_id(&self, _: &dyn mod_api::Host, event: &mut CreatorModIdEvent<'_>) {
        if event.stack.count() > 1 {
            *event.mod_id = Some("other_mod".to_owned());
        }
    }
}

/// A stick's Part: its stacks stand for no mod at all.
struct NoMod;

#[mod_api::horizon_part]
impl ItemPart for NoMod {
    fn creator_mod_id(&self, _: &dyn mod_api::Host, event: &mut CreatorModIdEvent<'_>) {
        *event.mod_id = None;
    }
}

/// The Part's answer wins for its own item, the namespace stands when it leaves it, and an item
/// without the Part reports its namespace. A Part may answer no mod, and an unknown item has no answer.
#[test]
#[ignore = "daily"]
fn a_part_names_the_mod_that_made_the_stack() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("creator", Box::new(Bucket))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            let creator = |game: &super::Game<'_>, item, count| {
                game.item_resource_creator_mod_id(&ItemStack::new(item, count))
            };
            assert_eq!(
                creator(game, "minecraft:bucket", 2).as_deref(),
                Some("other_mod")
            );
            assert_eq!(
                creator(game, "minecraft:bucket", 1).as_deref(),
                Some("minecraft")
            );
            assert_eq!(
                creator(game, "minecraft:stone", 1).as_deref(),
                Some("minecraft")
            );
            assert_eq!(creator(game, "minecraft:stick", 1), None);
            assert_eq!(creator(game, "creator:not_an_item", 1), None);
        });
    });
}
