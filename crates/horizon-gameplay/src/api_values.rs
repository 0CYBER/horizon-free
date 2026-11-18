//! The conversion boundary between mod API records and engine values: the traits and the
//! conversions that need no running game. horizon-client keeps the ones that name `Game` or its types.
use horizon_core::nbt::{Compound, JavaString, Tag};
use horizon_items::items::{ItemStack as EngineStack, Items};
use mod_api::item::item_stack::{ItemStack as ApiStack, StackWriteError};

pub mod block_edit;
pub mod block_entity_writeback;
pub mod block_state;
pub mod content_declarations;
pub mod damage;
pub mod game_rule_values;
pub mod gui_draw;
#[path = "api_values/ingredient.rs"]
mod ingredient_values;
pub mod inside_write;
pub mod item_model_context;
pub mod item_properties;
pub mod landing_write;
pub mod loot_values;
pub mod package_snapshot;
pub mod particle_values;
pub mod phase_values;
pub mod recipe_inputs;
pub mod small_values;
pub mod stack_count;
pub mod text_boundary;

pub use block_entity_writeback::{BlockEntitySnapshotError, finish_block_entity_writeback};
pub use stack_count::TryIntoEnginePreservingCount;

/// Convert an engine value into an owned API value after resolving registry identities.
pub trait TryIntoApi {
    type Api;
    type Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error>;
}

/// Validate a mod value against the game's registry before admitting it to engine state.
pub trait TryIntoEngine<Context: ?Sized = Items> {
    type Engine;
    type Error;
    fn try_into_engine(&self, context: &Context) -> Result<Self::Engine, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackReadError {
    UnknownItem,
    Empty,
    InvalidComponents(mod_api::StackError),
}

impl StackReadError {
    pub fn into_write_error(self) -> StackWriteError {
        match self {
            Self::UnknownItem => StackWriteError::UnknownItem,
            Self::Empty => StackWriteError::EmptyEngineStack,
            Self::InvalidComponents(reason) => StackWriteError::InvalidComponents { reason },
        }
    }
}

macro_rules! scalar_values {
    ($($ty:ty),* $(,)?) => { $(
        impl TryIntoApi for $ty {
            type Api = Self;
            type Error = StackReadError;
            fn try_into_api(&self) -> Result<Self, Self::Error> { Ok(*self) }
        }
        impl TryIntoEngine for $ty {
            type Engine = Self;
            type Error = StackWriteError;
            fn try_into_engine(&self, _: &Items) -> Result<Self, Self::Error> { Ok(*self) }
        }
    )* };
}
scalar_values!(u32, i32, bool, ());

impl TryIntoApi for String {
    type Api = Self;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self, Self::Error> {
        Ok(self.clone())
    }
}

impl<T: TryIntoApi + ?Sized> TryIntoApi for &T {
    type Api = T::Api;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        (*self).try_into_api()
    }
}
impl<C: ?Sized, T: TryIntoEngine<C> + ?Sized> TryIntoEngine<C> for &T {
    type Engine = T::Engine;
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        (*self).try_into_engine(context)
    }
}

impl<T: TryIntoApi> TryIntoApi for Option<T> {
    type Api = Option<T::Api>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.as_ref().map(TryIntoApi::try_into_api).transpose()
    }
}
impl<C: ?Sized, T: TryIntoEngine<C>> TryIntoEngine<C> for Option<T> {
    type Engine = Option<T::Engine>;
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        self.as_ref()
            .map(|value| value.try_into_engine(context))
            .transpose()
    }
}
impl<T: TryIntoApi, E: Clone> TryIntoApi for Result<T, E> {
    type Api = Result<T::Api, E>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        match self {
            Ok(value) => value.try_into_api().map(Ok),
            Err(error) => Ok(Err(error.clone())),
        }
    }
}

impl<T: TryIntoApi, const N: usize> TryIntoApi for [T; N] {
    type Api = [T::Api; N];
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut output: [Option<T::Api>; N] = std::array::from_fn(|_| None);
        for (target, source) in output.iter_mut().zip(self) {
            *target = Some(source.try_into_api()?);
        }
        Ok(output.map(|value| value.expect("every array element converted")))
    }
}
impl<C: ?Sized, T: TryIntoEngine<C>, const N: usize> TryIntoEngine<C> for [T; N] {
    type Engine = [T::Engine; N];
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        let mut output: [Option<T::Engine>; N] = std::array::from_fn(|_| None);
        for (target, source) in output.iter_mut().zip(self) {
            *target = Some(source.try_into_engine(context)?);
        }
        Ok(output.map(|value| value.expect("every array element converted")))
    }
}
impl<T: TryIntoApi> TryIntoApi for [T] {
    type Api = Vec<T::Api>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.iter().map(TryIntoApi::try_into_api).collect()
    }
}
impl<T: TryIntoApi> TryIntoApi for std::sync::Arc<[T]> {
    type Api = std::sync::Arc<[T::Api]>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(self.as_ref().try_into_api()?.into())
    }
}
impl<T: TryIntoApi> TryIntoApi for Vec<T> {
    type Api = Vec<T::Api>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.iter().map(TryIntoApi::try_into_api).collect()
    }
}
impl<C: ?Sized, T: TryIntoEngine<C>> TryIntoEngine<C> for Vec<T> {
    type Engine = Vec<T::Engine>;
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        self.as_slice().try_into_engine(context)
    }
}
impl<C: ?Sized, T: TryIntoEngine<C>> TryIntoEngine<C> for [T] {
    type Engine = Vec<T::Engine>;
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        self.iter()
            .map(|value| value.try_into_engine(context))
            .collect()
    }
}
impl TryIntoApi for (horizon_client::text::Text, u32) {
    type Api = (mod_api::Text, u32);
    type Error = mod_api::text::TextFormError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok((self.0.try_into_api()?, self.1))
    }
}
impl<C: ?Sized> TryIntoEngine<C> for (mod_api::Text, u32) {
    type Engine = (horizon_client::text::Text, u32);
    type Error = mod_api::text::TextFormError;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        Ok((self.0.try_into_engine(context)?, self.1))
    }
}

impl<A: TryIntoApi, B: TryIntoApi<Error = A::Error>> TryIntoApi for (A, B) {
    type Api = (A::Api, B::Api);
    type Error = A::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok((self.0.try_into_api()?, self.1.try_into_api()?))
    }
}
impl<C: ?Sized> TryIntoEngine<C> for (mod_api::Text, mod_api::Text) {
    type Engine = (horizon_client::text::Text, horizon_client::text::Text);
    type Error = mod_api::text::TextFormError;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        Ok((
            self.0.try_into_engine(context)?,
            self.1.try_into_engine(context)?,
        ))
    }
}

impl TryIntoEngine for mod_api::ItemDisplayContext {
    type Engine = horizon_assets::item_model::DisplayContext;
    type Error = ();
    fn try_into_engine(&self, _: &Items) -> Result<Self::Engine, Self::Error> {
        use horizon_assets::item_model::DisplayContext as Engine;
        use mod_api::ItemDisplayContext as Api;
        Ok(match self {
            Api::None => Engine::None,
            Api::ThirdPersonLeftHand => Engine::ThirdPersonLeftHand,
            Api::ThirdPersonRightHand => Engine::ThirdPersonRightHand,
            Api::FirstPersonLeftHand => Engine::FirstPersonLeftHand,
            Api::FirstPersonRightHand => Engine::FirstPersonRightHand,
            Api::Head => Engine::Head,
            Api::Gui => Engine::Gui,
            Api::Ground => Engine::Ground,
            Api::Fixed => Engine::Fixed,
            Api::OnShelf => Engine::OnShelf,
            _ => return Err(()),
        })
    }
}

impl TryIntoApi for horizon_client::merchant::ItemCost {
    type Api = mod_api::ItemCost;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::ItemCost::new(self.stack.try_into_api()?))
    }
}
impl TryIntoEngine for mod_api::ItemCost {
    type Engine = horizon_client::merchant::ItemCost;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            stack: self.stack.try_into_engine(items)?,
        })
    }
}
impl TryIntoApi for horizon_client::merchant::MerchantOffer {
    type Api = mod_api::MerchantOffer;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut offer = mod_api::MerchantOffer::new(
            self.base_cost_a.try_into_api()?,
            self.cost_b.try_into_api()?,
            self.result.try_into_api()?,
            self.max_uses,
            self.xp,
            self.price_multiplier,
        );
        offer.uses = self.uses;
        offer.reward_exp = self.reward_exp;
        offer.special_price_diff = self.special_price_diff;
        offer.demand = self.demand;
        Ok(offer)
    }
}
impl TryIntoEngine for mod_api::MerchantOffer {
    type Engine = horizon_client::merchant::MerchantOffer;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            base_cost_a: self.base_cost_a.try_into_engine(items)?,
            cost_b: self.cost_b.try_into_engine(items)?,
            result: self.result.try_into_engine(items)?,
            uses: self.uses,
            max_uses: self.max_uses,
            reward_exp: self.reward_exp,
            special_price_diff: self.special_price_diff,
            demand: self.demand,
            price_multiplier: self.price_multiplier,
            xp: self.xp,
        })
    }
}

impl TryIntoApi for EngineStack {
    type Api = ApiStack;
    type Error = StackReadError;

    fn try_into_api(&self) -> Result<ApiStack, Self::Error> {
        let key = mod_api::__private::item_key(&self.item).ok_or(StackReadError::UnknownItem)?;
        let mut result = ApiStack::new(key, self.count).ok_or(StackReadError::Empty)?;
        if self.carries_component(None, "minecraft:damage") {
            result.set_damage(self.damage);
        }
        if let Some(Tag::Compound(patch)) = horizon_items::item_data::encode_extra_components(self)
        {
            result
                .set_components(patch)
                .map_err(StackReadError::InvalidComponents)?;
        }
        Ok(result)
    }
}

impl TryIntoApi for horizon_assets::item_model::StackView {
    type Api = ApiStack;
    type Error = StackReadError;

    fn try_into_api(&self) -> Result<ApiStack, StackReadError> {
        let count = u32::try_from(self.count)
            .ok()
            .filter(|&count| count > 0)
            .ok_or(StackReadError::Empty)?;
        let components = Tag::compound(self.patch.iter().map(|(name, value)| match value {
            Some(value) => (
                horizon_core::nbt::JavaString::from(name.as_str()),
                horizon_client_core::scoreboard::json_to_tag(value),
            ),
            None => (
                horizon_core::nbt::JavaString::from(format!("!{name}").as_str()),
                Tag::Compound(Compound::new()),
            ),
        }));
        let key = mod_api::__private::item_key(&self.item).ok_or(StackReadError::UnknownItem)?;
        let mut stack = ApiStack::new(key, count).ok_or(StackReadError::Empty)?;
        let Tag::Compound(patch) = components else {
            unreachable!("model patch is constructed as a compound")
        };
        stack
            .set_components(patch)
            .map_err(StackReadError::InvalidComponents)?;
        Ok(stack)
    }
}

impl TryIntoEngine for mod_api::ItemResource {
    type Engine = EngineStack;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        self.as_stack().try_into_engine(items)
    }
}

/// Encode an API value for a read-only predicate or drawing, without imposing resident maxima.
/// Unknown keys and unrepresentable/invalid effect payloads still refuse conversion.
/// The write conversion below adds authoritative registry-dependent validation.
pub fn stack_value(value: &ApiStack) -> Result<EngineStack, StackWriteError> {
    let id = mod_api::__private::item_key_id(value.item()).ok_or(StackWriteError::UnknownItem)?;
    encode_stack(value, id.into())
}

fn encode_stack(
    value: &ApiStack,
    item: std::sync::Arc<str>,
) -> Result<EngineStack, StackWriteError> {
    let mut patch = value.components().map(|patch| patch.as_nbt().clone());
    // Let the existing component codec preserve explicit zero versus prototype inheritance.
    if value.damage_component() == Some(0) {
        patch
            .get_or_insert_with(Default::default)
            .insert(JavaString::from("minecraft:damage"), Tag::Int(0));
    }
    let patch = patch.map(Tag::Compound);
    let (_, extra) = horizon_items::item_data::decode_components(patch.as_ref());
    let stack = EngineStack {
        item,
        count: value.count(),
        damage: value.damage(),
        extra,
    };
    content_declarations::validate_decoded_effects(patch.as_ref(), &stack)?;
    Ok(stack)
}

impl TryIntoApi for horizon_client::crafting::RecipeRule {
    type Api = mod_api::RecipeRule;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match self {
            Self::Stonecutting { input, result } => mod_api::RecipeRule::Stonecutting {
                input: input.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::SmithingTransform {
                template,
                base,
                addition,
                result,
            } => mod_api::RecipeRule::SmithingTransform {
                template: template.try_into_api()?,
                base: base.try_into_api()?,
                addition: addition.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::SmithingTrim {
                template,
                base,
                addition,
                pattern,
            } => mod_api::RecipeRule::SmithingTrim {
                template: template.try_into_api()?,
                base: base.try_into_api()?,
                addition: addition.try_into_api()?,
                pattern: pattern.try_into_api()?,
            },
            Self::BannerDuplicate { banner, result } => mod_api::RecipeRule::BannerDuplicate {
                banner: banner.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::ShieldDecoration {
                banner,
                target,
                result,
            } => mod_api::RecipeRule::ShieldDecoration {
                banner: banner.try_into_api()?,
                target: target.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::Transmute {
                input,
                material,
                min,
                max,
                add_count,
                result,
            } => mod_api::RecipeRule::Transmute {
                input: input.try_into_api()?,
                material: material.try_into_api()?,
                min: *min,
                max: *max,
                add_count: *add_count,
                result: result.try_into_api()?,
            },
            Self::Dye {
                target,
                dye,
                result,
            } => mod_api::RecipeRule::Dye {
                target: target.try_into_api()?,
                dye: dye.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::Imbue {
                source,
                material,
                result,
            } => mod_api::RecipeRule::Imbue {
                source: source.try_into_api()?,
                material: material.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::BookCloning {
                source,
                material,
                min,
                max,
                result,
            } => mod_api::RecipeRule::BookCloning {
                source: source.try_into_api()?,
                material: material.try_into_api()?,
                min: *min,
                max: *max,
                result: result.try_into_api()?,
            },
            Self::FireworkRocket {
                shell,
                fuel,
                star,
                result,
            } => mod_api::RecipeRule::FireworkRocket {
                shell: shell.try_into_api()?,
                fuel: fuel.try_into_api()?,
                star: star.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::FireworkStar {
                dye,
                fuel,
                trail,
                twinkle,
                shapes,
                result,
            } => mod_api::RecipeRule::FireworkStar {
                dye: dye.try_into_api()?,
                fuel: fuel.try_into_api()?,
                trail: trail.try_into_api()?,
                twinkle: twinkle.try_into_api()?,
                shapes: shapes.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::FireworkStarFade {
                dye,
                target,
                result,
            } => mod_api::RecipeRule::FireworkStarFade {
                dye: dye.try_into_api()?,
                target: target.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::MapExtending {
                map,
                material,
                result,
            } => mod_api::RecipeRule::MapExtending {
                map: map.try_into_api()?,
                material: material.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::DecoratedPot {
                back,
                left,
                right,
                front,
                result,
            } => mod_api::RecipeRule::DecoratedPot {
                back: back.try_into_api()?,
                left: left.try_into_api()?,
                right: right.try_into_api()?,
                front: front.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::RepairItem => mod_api::RecipeRule::RepairItem,
        })
    }
}

impl TryIntoApi for horizon_client::crafting::RecipeTemplate {
    type Api = mod_api::RecipeTemplate;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::RecipeTemplate::new(
            self.item.clone(),
            self.count,
            self.components.clone(),
        ))
    }
}
impl TryIntoApi for horizon_client::crafting::RecipeTrimPattern {
    type Api = mod_api::RecipeTrimPattern;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match self {
            Self::Reference(id) => mod_api::RecipeTrimPattern::Reference(id.clone()),
            Self::Inline {
                asset_id,
                description,
                decal,
            } => mod_api::RecipeTrimPattern::Inline {
                asset_id: asset_id.clone(),
                description: description.clone(),
                decal: *decal,
            },
        })
    }
}
impl TryIntoApi for horizon_client::crafting::FireworkShapeIngredient {
    type Api = mod_api::FireworkShapeIngredient;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::FireworkShapeIngredient::new(
            self.shape.try_into_api().unwrap(),
            self.ingredient.try_into_api()?,
        ))
    }
}
impl TryIntoApi for horizon_client::crafting::RecipeSnapshot {
    type Api = mod_api::Recipe;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut result =
            mod_api::Recipe::new(self.id.clone(), self.kind.clone(), self.json.clone());
        result.group = self.group.clone();
        result.ingredients = self.ingredients.try_into_api()?;
        result.results = self.results.try_into_api()?;
        result.cooking_time = self.cooking_time;
        result.experience = self.experience;
        result.rule = self.rule.try_into_api()?;
        Ok(result)
    }
}

impl TryIntoApi for horizon_client::stations::AnvilResult {
    type Api = mod_api::item::stations::AnvilResult;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::item::stations::AnvilResult {
            result: self.result.try_into_api()?,
            cost: self.cost,
            repair_item_count_cost: self.repair_item_count_cost,
            only_renaming: self.only_renaming,
        })
    }
}
impl TryIntoEngine for mod_api::item::stations::AnvilResult {
    type Engine = horizon_client::stations::AnvilResult;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(horizon_client::stations::AnvilResult {
            result: self.result.try_into_engine(items)?,
            cost: self.cost,
            repair_item_count_cost: self.repair_item_count_cost,
            only_renaming: self.only_renaming,
        })
    }
}
impl TryIntoApi for horizon_client::menu::AnvilCraft {
    type Api = mod_api::item::menu::AnvilCraft;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::item::menu::AnvilCraft {
            left: self.left.try_into_api()?,
            right: self.right.try_into_api()?,
            output: self.output.try_into_api()?,
        })
    }
}
impl TryIntoEngine for mod_api::item::menu::AnvilCraft {
    type Engine = horizon_client::menu::AnvilCraft;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(horizon_client::menu::AnvilCraft {
            left: self.left.try_into_engine(items)?,
            right: self.right.try_into_engine(items)?,
            output: self.output.try_into_engine(items)?,
        })
    }
}
impl TryIntoApi for horizon_client::menu::GrindstonePlace {
    type Api = mod_api::item::menu::GrindstonePlace;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::item::menu::GrindstonePlace {
            top: self.top.try_into_api()?,
            bottom: self.bottom.try_into_api()?,
            vanilla_output: self.vanilla_output.try_into_api()?,
            output: self.output.try_into_api()?,
            xp: self.xp,
        })
    }
}
impl TryIntoEngine for mod_api::item::menu::GrindstonePlace {
    type Engine = horizon_client::menu::GrindstonePlace;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(horizon_client::menu::GrindstonePlace {
            top: self.top.try_into_engine(items)?,
            bottom: self.bottom.try_into_engine(items)?,
            vanilla_output: self.vanilla_output.try_into_engine(items)?,
            output: self.output.try_into_engine(items)?,
            xp: self.xp,
        })
    }
}
impl TryIntoApi for horizon_client::menu::GrindstoneTake {
    type Api = mod_api::item::menu::GrindstoneTake;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::item::menu::GrindstoneTake {
            top: self.top.try_into_api()?,
            bottom: self.bottom.try_into_api()?,
            xp: self.xp,
            new_top: self.new_top.try_into_api()?,
            new_bottom: self.new_bottom.try_into_api()?,
        })
    }
}
impl TryIntoEngine for mod_api::item::menu::GrindstoneTake {
    type Engine = horizon_client::menu::GrindstoneTake;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(horizon_client::menu::GrindstoneTake {
            top: self.top.try_into_engine(items)?,
            bottom: self.bottom.try_into_engine(items)?,
            xp: self.xp,
            new_top: self.new_top.try_into_engine(items)?,
            new_bottom: self.new_bottom.try_into_engine(items)?,
        })
    }
}

impl TryIntoApi for horizon_client::menu::AnvilUpdate {
    type Api = mod_api::item::menu::AnvilUpdate;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let vanilla = mod_api::item::stations::AnvilResult {
            result: self.vanilla.output.try_into_api()?,
            cost: self.vanilla.xp_cost,
            repair_item_count_cost: self.vanilla.material_cost.max(0) as u32,
            only_renaming: false,
        };
        let mut result = mod_api::item::menu::AnvilUpdate::new(
            self.left.try_into_api()?,
            self.right.try_into_api()?,
            self.name.clone(),
            &vanilla,
        );
        result.output = self.output.try_into_api()?;
        result.xp_cost = self.xp_cost;
        result.material_cost = self.material_cost;
        Ok(result)
    }
}

impl TryIntoEngine for mod_api::item::menu::AnvilUpdate {
    type Engine = horizon_client::menu::AnvilUpdate;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(horizon_client::menu::AnvilUpdate {
            left: self.left.try_into_engine(items)?,
            right: self.right.try_into_engine(items)?,
            name: self.name.clone(),
            vanilla: horizon_client::menu::AnvilVanilla {
                output: self.vanilla.output.try_into_engine(items)?,
                xp_cost: self.vanilla.xp_cost,
                material_cost: self.vanilla.material_cost,
            },
            output: self.output.try_into_engine(items)?,
            xp_cost: self.xp_cost,
            material_cost: self.material_cost,
        })
    }
}
impl TryIntoApi for horizon_client::containers::ModBlockEntityFields {
    type Api = mod_api::item::containers::ModBlockEntityFields;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::item::containers::ModBlockEntityFields {
            nbt: self.nbt.clone(),
            items: self.items.try_into_api()?,
            data: self.data.clone(),
        })
    }
}
impl TryIntoEngine for mod_api::item::containers::ModBlockEntityFields {
    type Engine = horizon_client::containers::ModBlockEntityFields;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(horizon_client::containers::ModBlockEntityFields {
            nbt: self.nbt.clone(),
            items: self.items.try_into_engine(items)?,
            data: self.data.clone(),
        })
    }
}
impl TryIntoApi for horizon_client::containers::ModBlockEntity {
    type Api = mod_api::ModBlockEntity;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::ModBlockEntity::from_snapshot(
            self.id().to_owned(),
            (**self).try_into_api()?,
            mod_api::item::containers::ModRevision::from_snapshot(
                self.revision().identity(),
                self.revision().changes(),
            ),
        ))
    }
}

impl TryIntoApi for horizon_client_core::suggestion::Suggestion {
    type Api = mod_api::Suggestion;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut value = mod_api::Suggestion::new(self.range, self.text.clone());
        value.tooltip.clone_from(&self.tooltip);
        value.int_value = self.int_value;
        Ok(value)
    }
}
impl TryIntoEngine for mod_api::Suggestion {
    type Engine = horizon_client_core::suggestion::Suggestion;
    type Error = StackWriteError;
    fn try_into_engine(&self, _: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            range: self.range,
            text: self.text.clone(),
            tooltip: self.tooltip.clone(),
            int_value: self.int_value,
        })
    }
}
// Registration declarations do not need a live item registry.
impl TryIntoEngine<()> for mod_api::BlockEntityTypeDecl {
    type Engine = horizon_mod_package::content::BlockEntityType;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            id: self.id.clone(),
            blocks: self.blocks.clone(),
        })
    }
}
impl TryIntoApi for horizon_world::level::PlacementContext {
    type Api = mod_api::PlacementContext;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let item = if self.item.is_empty() {
            None
        } else {
            Some(mod_api::__private::item_key(&self.item).ok_or(StackReadError::UnknownItem)?)
        };
        let mut snapshot = mod_api::PlacementContext::new(self.pos, self.face, item);
        snapshot.hit_pos = self.hit_pos;
        snapshot.hit = [self.hit.x, self.hit.y, self.hit.z];
        snapshot.inside = self.inside;
        snapshot.yaw = self.yaw;
        snapshot.pitch = self.pitch;
        snapshot.sneaking = self.sneaking;
        snapshot.replace_clicked = self.replace_clicked;
        Ok(snapshot)
    }
}

impl TryIntoApi for horizon_client_core::gamerules::Value {
    type Api = mod_api::GameRuleValue;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match *self {
            Self::Bool(value) => mod_api::GameRuleValue::Bool(value),
            Self::Int(value) => mod_api::GameRuleValue::Int(value),
        })
    }
}

impl TryIntoEngine for mod_api::GameRuleValue {
    type Engine = horizon_client_core::gamerules::Value;
    type Error = ();
    fn try_into_engine(&self, _: &Items) -> Result<Self::Engine, Self::Error> {
        match *self {
            Self::Bool(value) => Ok(Self::Engine::Bool(value)),
            Self::Int(value) => Ok(Self::Engine::Int(value)),
            // A kind this game has no rule of.
            _ => Err(()),
        }
    }
}

impl TryIntoApi for horizon_client::boss_overlay::ModBar {
    type Api = mod_api::ClientBossBar;
    type Error = mod_api::text::TextFormError;

    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::ClientBossBar {
            uuid: self.uuid,
            name: self.name.try_into_api()?,
            progress: self.progress,
            color: self.color,
            overlay: self.overlay,
            darken_screen: self.darken_screen,
            play_boss_music: self.play_boss_music,
            create_world_fog: self.create_world_fog,
        })
    }
}

impl TryIntoApi for horizon_world::chunk::BlockState {
    type Api = mod_api::BlockState;
    type Error = mod_api::BlockStateError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let block = mod_api::__private::block_key(&self.name)
            .ok_or(mod_api::BlockStateError::UnknownBlock)?;
        let properties: Vec<_> = self
            .properties
            .iter()
            .map(|(name, value)| mod_api::StateConstraint::new(name.clone(), value.clone()))
            .collect();
        mod_api::BlockState::of(block_metadata_table()?, block, &properties)
    }
}

pub fn block_metadata_table()
-> Result<&'static horizon_mod_runtime::block_definitions::Blocks, mod_api::BlockStateError> {
    mod_api::__private::block_definitions().ok_or(mod_api::BlockStateError::Unavailable)
}
