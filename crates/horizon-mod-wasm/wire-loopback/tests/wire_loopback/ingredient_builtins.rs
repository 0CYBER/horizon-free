//! Ingredient records retain built-in predicates and custom slot displays on the host boundary.
use super::*;
struct World;
fn ingredient() -> mod_api::Ingredient {
    let mut base = mod_api::Ingredient::of(vec!["minecraft:diamond_sword".into()]);
    base.rule = Some(Box::new(
        mod_api::item::crafting::IngredientRule::Components {
            patch: [("minecraft:damage".into(), mod_api::Tag::Int(5))].into(),
            removed: vec!["minecraft:enchantments".into()],
            strict: true,
        },
    ));
    let mut shown = mod_api::Ingredient::of(vec!["minecraft:diamond_sword".into()]);
    shown.children = Some((mod_api::item::crafting::Shape::CustomDisplay, vec![base]));
    shown.display = Some(Box::new(
        [
            ("type".into(), mod_api::Tag::String("minecraft:item".into())),
            (
                "item".into(),
                mod_api::Tag::String("minecraft:diamond".into()),
            ),
        ]
        .into(),
    ));
    shown
}
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World, except(recipes));
impl mod_api::prelude::RecipesHost for World {
    fn recipe_ingredient(
        &self,
        json: &str,
    ) -> Result<Option<mod_api::Ingredient>, mod_api::RecipeJsonError> {
        assert_eq!(json, "fixture:ingredient");
        Ok(Some(ingredient()))
    }
}
#[test]
fn ingredient_components_and_custom_display_cross_the_host_wire() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let world = World;
    let mut frame = boundary::Frame::default();
    frame.set_host(&world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let id = boundary::import_id("Host", "recipe_ingredient").unwrap();
    let mut input = Writer::new();
    input.put(&"fixture:ingredient".to_owned());
    let mut output = Writer::new();
    unsafe { boundary::serve(id, &mut cx, input.bytes(), &mut output) }.unwrap();
    let decoded = Reader::decode::<Result<Option<mod_api::Ingredient>, mod_api::RecipeJsonError>>(
        output.bytes(),
    )
    .unwrap();
    assert_eq!(decoded.unwrap(), Some(ingredient()));
}
