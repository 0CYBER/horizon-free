//! Potions and the brewing stand: the `Potions` / `MobEffects` registries (code
//! in vanilla, ported as tables), data-driven `minecraft:brewing` recipes from
//! `data/minecraft/recipe/brewing`, potion colours and tooltips, and
//! `BrewingStandBlockEntity.serverTick`.
pub use crate::chunk::block_entity::BrewingStand;
use crate::{
    items::{ItemStack, Items},
    providers::Providers,
};
use horizon_core::identifier::parse_or_keep;
use horizon_world::loot::PackOverlay;
use serde_json::Value;
use std::{collections::HashSet, path::Path};

pub use horizon_client_core::potions::{
    BASE_POTION_COLOR, Category, EFFECTS, EffectDef, POTIONS, PotionDef, all_effects,
    description_id, effect, is_mod_effect, mod_effects, namespaced, potion, potion_color,
    register_mod_effects, set_pack_colors,
};
/// `MobEffectUtil.formatDuration` at 20 TPS.
pub fn format_duration(ticks: i32) -> String {
    format_tick_duration(ticks, 20.0)
}
/// `StringUtil.formatTickDuration`: mm:ss, or hh:mm:ss from an hour up, every
/// field two digits.
pub fn format_tick_duration(ticks: i32, tick_rate: f32) -> String {
    let secs = (ticks as f32 / tick_rate).floor() as i32;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}
/// `item.minecraft.potion.effect.<name>` style translation key for a potion item.
pub fn potion_name_key(item: &str, potion: Option<&str>) -> String {
    let prefix = item.trim_start_matches("minecraft:");
    let suffix = potion.and_then(self::potion).map_or("empty", |p| p.name);
    format!("item.minecraft.{prefix}.effect.{suffix}")
}
/// `PotionItem.getName` and `TippedArrowItem.getName`: a potion or a tipped
/// arrow is named by its contents (`PotionContents.getName`: their own
/// `custom_name`, else the potion's name, else `empty`), whatever its
/// `item_name`. None for other items, and for a stack whose contents were
/// taken off (`!minecraft:potion_contents`), which takes the item's name.
pub fn contents_name_key(stack: &ItemStack) -> Option<String> {
    use horizon_core::nbt::Tag;
    let item = &*stack.item;
    let named = matches!(
        item,
        "minecraft:potion"
            | "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:tipped_arrow"
    );
    let data = stack.data();
    if !named || data.is_some_and(|d| d.other("!minecraft:potion_contents").is_some()) {
        return None;
    }
    let prefix = item.trim_start_matches("minecraft:");
    let own = data
        .and_then(|d| d.potion_extra.iter().find(|(k, _)| k == "custom_name"))
        .and_then(|(_, name)| match name {
            Tag::String(s) => Some(s.to_string_lossy()),
            _ => None,
        });
    Some(match own {
        Some(suffix) => format!("item.minecraft.{prefix}.effect.{suffix}"),
        None => potion_name_key(item, stack.potion()),
    })
}
/// `PotionContents.getAllEffects`: the potion's effects, then the stack's
/// `custom_effects` (id, duration, amplifier). Unknown effect ids are dropped.
pub fn contents_effects(stack: &ItemStack) -> Vec<(&'static str, i32, i32)> {
    let potion = stack.potion().and_then(self::potion);
    let custom = stack.custom_potion_effects();
    potion
        .map_or(&[][..], |d| d.effects)
        .iter()
        .copied()
        .chain(
            custom
                .iter()
                .filter_map(|(id, duration, amp)| Some((effect(id)?.id, *duration, *amp))),
        )
        .collect()
}
/// `PotionContents.getAllEffects`, retaining each custom instance's flags.
pub fn contents_instances(stack: &ItemStack) -> Vec<crate::effects::EffectInstance> {
    use horizon_client_core::nbt_value::{number, text};
    use horizon_core::nbt::Tag;
    let mut effects: Vec<_> = stack
        .potion()
        .and_then(potion)
        .map_or(&[][..], |p| p.effects)
        .iter()
        .map(|(id, duration, amp)| crate::effects::EffectInstance::new(id, *duration, *amp))
        .collect();
    if let Some((_, Tag::List(custom))) = stack
        .data()
        .and_then(|d| d.potion_extra.iter().find(|(k, _)| k == "custom_effects"))
    {
        for e in custom {
            let Some(def) = text(e.get("id")).and_then(|id| effect(&id)) else {
                continue;
            };
            let mut instance = crate::effects::EffectInstance::new(
                def.id,
                number(e.get("duration")).unwrap_or(0) as i32,
                number(e.get("amplifier")).unwrap_or(0).clamp(0, 255) as i32,
            );
            instance.ambient = number(e.get("ambient")).unwrap_or(0) != 0;
            instance.visible = number(e.get("show_particles")) != Some(0);
            effects.push(instance);
        }
    }
    effects
}
/// `PotionContents.getColor`: an explicit colour wins; otherwise mix only
/// visible base and custom effects, with their amplifier weights.
pub fn contents_color(stack: &ItemStack) -> u32 {
    use horizon_client_core::nbt_value::number;
    if let Some(color) = stack
        .data()
        .and_then(|d| d.potion_extra.iter().find(|(k, _)| k == "custom_color"))
        .and_then(|(_, v)| number(Some(v)))
    {
        return color as u32;
    }
    0xFF00_0000
        | horizon_client_core::potions::effects_color(
            contents_instances(stack)
                .iter()
                .filter(|e| e.visible)
                .map(|e| (e.id, e.amplifier)),
        )
}
/// `PotionContents.equals(EMPTY)`: colour and custom name also make contents
/// nonempty, even when there is no registered potion or custom effect.
pub fn contents_empty(stack: &ItemStack) -> bool {
    use horizon_core::nbt::Tag;
    stack.potion().is_none()
        && stack.data().is_none_or(|d| {
            d.potion_extra
                .iter()
                .all(|(key, value)| match key.as_str() {
                    "custom_effects" => matches!(value, Tag::List(list) if list.is_empty()),
                    "custom_color" | "custom_name" => false,
                    _ => true,
                })
        })
}
/// Roman amplifier suffix (`potion.potency.N`) and duration lines of a potion's
/// effects and the stack's custom ones (`PotionContents.getAllEffects`).
pub fn tooltip(
    stack: &ItemStack,
    scale: f32,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, Category)> {
    let effects = contents_effects(stack);
    if effects.is_empty() {
        if stack.potion().and_then(self::potion).is_none() {
            return vec![];
        }
        return vec![(
            translate("effect.none").unwrap_or_else(|| "No Effects".into()),
            Category::Neutral,
        )];
    }
    effect_lines(effects, scale, translate)
}
/// `PotionContents.addPotionTooltip`'s line per effect: its name, potency and
/// scaled duration, for (effect id, duration, amplifier) entries.
pub fn effect_lines<'a>(
    effects: impl IntoIterator<Item = (&'a str, i32, i32)>,
    scale: f32,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, Category)> {
    effects
        .into_iter()
        .map(|(e, duration, amp)| {
            let e = e.trim_start_matches("minecraft:");
            let mut line = translate(&description_id(e)).unwrap_or_else(|| e.to_owned());
            if amp > 0 {
                line.push(' ');
                line.push_str(
                    &translate(&format!("potion.potency.{amp}"))
                        .unwrap_or_else(|| crate::enchant::level_name(amp as u32 + 1)),
                );
            }
            let instant = effect(e).is_some_and(|d| d.instant);
            if !instant {
                if duration == -1 {
                    // MobEffectInstance.isInfiniteDuration: formatDuration says
                    // the infinite sign, not a time.
                    line = format!(
                        "{line} ({})",
                        translate("effect.duration.infinite").unwrap_or_else(|| "∞".to_owned())
                    );
                } else {
                    let scaled = (duration as f32 * scale) as i32;
                    if scaled > 20 {
                        line = format!("{line} ({})", format_duration(scaled));
                    }
                }
            }
            (line, effect(e).map_or(Category::Neutral, |d| d.category))
        })
        .collect()
}

/// A `PotionIngredient`: item ids plus an optional potion filter.
#[derive(Clone, Debug)]
pub struct PotionIngredient {
    pub items: HashSet<String>,
    pub potions: Option<HashSet<String>>,
}
impl PotionIngredient {
    fn parse(v: &Value, items: &Items) -> Self {
        let mut set = HashSet::new();
        // `Ingredient.CODEC` reads each id through `HolderSetCodec` (`TagKey.hashedCodec`
        // for tags, `Item.CODEC` for items): a bare id or `#tag` names `minecraft`'s.
        let add = |s: &str, set: &mut HashSet<String>| match s.strip_prefix('#') {
            Some(tag) => {
                set.extend(
                    items
                        .item_tags
                        .get(&parse_or_keep(tag))
                        .into_iter()
                        .flatten()
                        .cloned(),
                );
            }
            None => {
                set.insert(parse_or_keep(s));
            }
        };
        match &v["item"] {
            Value::String(s) => add(s, &mut set),
            Value::Array(a) => a
                .iter()
                .filter_map(Value::as_str)
                .for_each(|s| add(s, &mut set)),
            _ => {}
        }
        let potions = match &v["potion_contents"]["potions"] {
            Value::String(s) => Some([s.clone()].into_iter().collect()),
            Value::Array(a) => Some(
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect(),
            ),
            _ => None,
        };
        Self {
            items: set,
            potions,
        }
    }
    pub fn test(&self, stack: &ItemStack) -> bool {
        self.items.contains(&*stack.item)
            && self
                .potions
                .as_ref()
                .is_none_or(|p| stack.potion().is_some_and(|x| p.contains(x)))
    }
}
#[derive(Clone, Debug)]
pub struct BrewingRecipe {
    pub id: String,
    pub input: PotionIngredient,
    pub reagent: PotionIngredient,
    pub output: ItemStack,
}
pub struct Brewing {
    pub recipes: Vec<BrewingRecipe>,
    /// `RecipePropertySet.BREWING_REAGENTS`.
    pub reagents: HashSet<String>,
    /// Items accepted in the bottle slots (`PotionIngredient.isPotionInput`).
    pub inputs: HashSet<String>,
}
impl Brewing {
    pub fn load(root: &Path, items: &Items, overlay: &PackOverlay) -> Self {
        let mut sources =
            horizon_items::recipe_sources::sources(root, overlay).unwrap_or_else(|e| {
                log::warn!("Horizon: brewing recipes not loaded: {e}");
                Vec::new()
            });
        sources.retain(|(id, _)| {
            let path = id.split_once(':').map_or(id.as_str(), |(_, path)| path);
            path.strip_prefix("brewing/")
                .is_some_and(|name| !name.contains('/'))
        });
        for (id, _) in &mut sources {
            *id = horizon_items::recipe_sources::qualified(id);
        }
        sources.sort_by(|a, b| a.0.cmp(&b.0));
        let mut recipes = Vec::new();
        for (id, v) in sources {
            if v["type"] != "minecraft:brewing" {
                continue;
            }
            // BrewingRecipe.output is an ItemStackTemplate: decode the whole
            // stack, not just its potion id, so custom_name, custom_effects
            // and component removals from a data pack survive the brew.
            let tag = horizon_client_core::scoreboard::json_to_tag(&v["output"]);
            let output = crate::item_data::template_from(&tag)
                .unwrap_or_else(|| ItemStack::new("minecraft:potion", 1));
            recipes.push(BrewingRecipe {
                id,
                input: PotionIngredient::parse(&v["input"], items),
                reagent: PotionIngredient::parse(&v["reagent"], items),
                output,
            });
        }
        let reagents = recipes
            .iter()
            .flat_map(|r| r.reagent.items.iter().cloned())
            .collect();
        let inputs = recipes
            .iter()
            .flat_map(|r| r.input.items.iter().cloned())
            .chain(std::iter::once("minecraft:glass_bottle".to_owned()))
            .collect();
        Self {
            recipes,
            reagents,
            inputs,
        }
    }
    pub fn find(&self, input: &ItemStack, reagent: &ItemStack) -> Option<&BrewingRecipe> {
        self.recipes
            .iter()
            .find(|r| r.input.test(input) && r.reagent.test(reagent))
    }
    pub fn is_reagent(&self, stack: &ItemStack) -> bool {
        self.reagents.contains(&*stack.item)
    }
    pub fn is_bottle(&self, stack: &ItemStack) -> bool {
        self.inputs.contains(&*stack.item)
    }
}

/// What a brewing tick asks the world to do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrewTick {
    /// New `has_bottle_0..2` block state values.
    pub bottles: Option<[bool; 3]>,
    /// Level event 1035 (brew sound).
    pub brewed: bool,
    /// A fuel item was taken: `serverTick` marks the stand changed for it even
    /// when its `brewing_fuel` gave no uses.
    pub fuel_taken: bool,
    /// Remainders dropped at the stand.
    pub drops: Vec<ItemStack>,
}
impl BrewingStand {
    pub fn new() -> Self {
        Self {
            total_brew_time: 400,
            total_fuel: 20,
            speed: 1.0,
            ..Default::default()
        }
    }
    fn brewable(&self, brewing: &Brewing) -> bool {
        let Some(ingredient) = &self.items[3] else {
            return false;
        };
        if !brewing.is_reagent(ingredient) {
            return false;
        }
        self.items[..3]
            .iter()
            .flatten()
            .any(|b| brewing.find(b, ingredient).is_some())
    }
    /// `BrewingStandBlockEntity.serverTick`.
    pub fn tick(
        &mut self,
        brewing: &Brewing,
        items: &Items,
        providers: &Providers,
        remainder: &dyn Fn(&ItemStack) -> Option<ItemStack>,
    ) -> BrewTick {
        let (mut out, due) = self.tick_to_brew(brewing, items, providers, remainder);
        if due {
            self.brew(brewing, remainder, &mut out);
        }
        self.sync_bottles(&mut out);
        out
    }
    /// `serverTick` up to its `doBrew` call: true when the brew is due now.
    /// A caller asking mods around `doBrew` runs [`Self::brew`] itself, then
    /// [`Self::sync_bottles`].
    pub fn tick_to_brew(
        &mut self,
        brewing: &Brewing,
        items: &Items,
        providers: &Providers,
        remainder: &dyn Fn(&ItemStack) -> Option<ItemStack>,
    ) -> (BrewTick, bool) {
        let mut out = BrewTick::default();
        if self.fuel <= 0
            && let Some(fuel) = self.items[4].as_mut()
            && let Some((uses, speed)) = brewing_fuel(items, fuel)
        {
            self.fuel = providers.int_value(&uses);
            self.total_fuel = self.fuel;
            self.speed = providers.float_value(&speed, 1.0);
            let remainder = remainder(fuel);
            fuel.count -= 1;
            if fuel.count == 0 {
                self.items[4] = remainder;
            } else if let Some(r) = remainder {
                out.drops.push(r);
            }
            out.fuel_taken = true;
        }
        let brewable = self.brewable(brewing);
        let mut due = false;
        if self.brew_time > 0 {
            self.brew_time -= 1;
            let same = self.items[3].as_ref().map(|s| s.item.clone()) == self.ingredient;
            if self.brew_time == 0 && brewable {
                due = true;
            } else if !brewable || !same {
                self.brew_time = 0;
            }
        } else if brewable && self.fuel > 0 {
            let speed = if self.speed > 0.0 { self.speed } else { 1.0 };
            self.fuel -= 1;
            self.brew_time = (400.0 / speed).ceil() as i32;
            self.total_brew_time = self.brew_time;
            self.ingredient = self.items[3].as_ref().map(|s| s.item.clone());
        }
        (out, due)
    }
    /// The end of `serverTick`: the `has_bottle_*` block state when the
    /// bottle slots filled or emptied.
    pub fn sync_bottles(&mut self, out: &mut BrewTick) {
        let bottles = [
            self.items[0].is_some(),
            self.items[1].is_some(),
            self.items[2].is_some(),
        ];
        if self.last_bottles != Some(bottles) {
            self.last_bottles = Some(bottles);
            out.bottles = Some(bottles);
        }
    }
    pub fn idle(&self) -> bool {
        let bottles = [
            self.items[0].is_some(),
            self.items[1].is_some(),
            self.items[2].is_some(),
        ];
        self.brew_time <= 0
            && self.items[3].is_none()
            && (self.fuel > 0 || self.items[4].is_none())
            && self.last_bottles == Some(bottles)
    }
    /// `doBrew` once `PotionBrewEvent.Pre` asked the mods with `asked`, a
    /// copy of the five slots, and whether they cancelled: a cancel puts the
    /// mods' slots in and skips the brew (the caller then fires Post);
    /// otherwise vanilla brews and the copy is dropped. Without mods (`None`)
    /// this is `doBrew`.
    pub fn brew_after_pre(
        &mut self,
        asked: Option<([Option<ItemStack>; 5], bool)>,
        brewing: &Brewing,
        remainder: &dyn Fn(&ItemStack) -> Option<ItemStack>,
        out: &mut BrewTick,
    ) {
        match asked {
            Some((stacks, true)) => self.items = stacks,
            _ => self.brew(brewing, remainder, out),
        }
    }
    /// `BrewingStandBlockEntity.doBrew`.
    pub fn brew(
        &mut self,
        brewing: &Brewing,
        remainder: &dyn Fn(&ItemStack) -> Option<ItemStack>,
        out: &mut BrewTick,
    ) {
        let Some(ingredient) = self.items[3].clone() else {
            return;
        };
        for slot in 0..3 {
            if let Some(input) = &self.items[slot]
                && let Some(r) = brewing.find(input, &ingredient)
            {
                self.items[slot] = Some(r.output.clone());
            }
        }
        let remainder = remainder(&ingredient);
        if let Some(i) = self.items[3].as_mut() {
            i.count -= 1;
            if i.count == 0 {
                self.items[3] = remainder;
            } else if let Some(r) = remainder {
                out.drops.push(r);
            }
        }
        out.brewed = true;
    }
}
/// `ItemStack.get(BREWING_FUEL)`: the stack's providers (uses, speed_multiplier),
/// respecting component patches before falling back to the item's defaults.
pub fn brewing_fuel(items: &Items, stack: &ItemStack) -> Option<(Value, Value)> {
    if let Some(data) = stack.data() {
        if data.other("!minecraft:brewing_fuel").is_some() {
            return None;
        }
        if let Some(own) = data.other("minecraft:brewing_fuel") {
            use horizon_core::nbt::Tag;
            let uses = match own.get("uses")? {
                Tag::Byte(n) => Value::from(*n),
                Tag::Short(n) => Value::from(*n),
                Tag::Int(n) => Value::from(*n),
                Tag::Long(n) => Value::from(*n),
                // Named and structured providers remain supported.
                provider => horizon_core::json_nbt::tag_to_json(provider).ok()?,
            };
            let speed = horizon_core::json_nbt::tag_to_json(own.get("speed_multiplier")?).ok()?;
            return Some((uses, speed));
        }
    }
    items.get(&stack.item).and_then(|i| i.brewing_fuel.clone())
}

#[cfg(test)]
mod tests {
    fn data_remainder(items: &super::Items, stack: &super::ItemStack) -> Option<super::ItemStack> {
        items
            .get(&stack.item)?
            .crafting_remainder
            .as_deref()
            .map(|id| super::ItemStack::new(id, 1))
    }

    use super::*;

    fn content_brewing_root(name: &str, condition: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "horizon-content-brewing-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        std::fs::create_dir_all(root.join("data/example/recipe/brewing")).unwrap();
        std::fs::write(
            root.join(horizon_items::content_namespaces::MARKER),
            br#"["example"]"#,
        )
        .unwrap();
        std::fs::write(root.join("data/example/recipe/brewing/test.json"), format!(r#"{{"type":"minecraft:brewing","input":{{"item":"minecraft:potion"}},"reagent":{{"item":"minecraft:nether_wart"}},"output":{{"id":"minecraft:diamond"}}{condition}}}"#)).unwrap();
        root
    }

    #[test]
    fn crafting_remainder_brewing_fuel_and_ingredient_use_current_stack() {
        use horizon_core::nbt::Tag;
        let items = Items::for_tests(Default::default());
        let brewing = Brewing {
            recipes: Vec::new(),
            reagents: HashSet::new(),
            inputs: HashSet::new(),
        };
        let mut fuel = ItemStack {
            damage: 2,
            ..ItemStack::new("test:tool", 1)
        };
        fuel.data_mut().set_loot_component(
            "minecraft:brewing_fuel",
            &Some(Tag::compound([
                ("uses", Tag::Int(7)),
                ("speed_multiplier", Tag::Float(1.0_f32.to_bits())),
            ])),
        );
        let worn = |s: &ItemStack| {
            Some(ItemStack {
                count: 1,
                damage: s.damage + 1,
                ..s.clone()
            })
        };
        let mut stand = BrewingStand::new();
        stand.items[4] = Some(fuel);
        stand.tick_to_brew(&brewing, &items, &Providers::empty(), &worn);
        assert_eq!(
            stand.items[4].as_ref().map(|s| (s.damage, s.count)),
            Some((3, 1))
        );
        assert_eq!(stand.fuel, 7);
        stand.items[3] = Some(ItemStack {
            damage: 2,
            ..ItemStack::new("test:tool", 1)
        });
        let mut out = BrewTick::default();
        stand.brew(&brewing, &worn, &mut out);
        assert_eq!(
            stand.items[3].as_ref().map(|s| (s.damage, s.count)),
            Some((3, 1))
        );
        assert!(out.brewed);
        assert!(out.drops.is_empty());
    }

    #[test]
    fn content_brewing_recipe_is_loaded() {
        let root = content_brewing_root("loaded", "");
        std::fs::create_dir_all(root.join("data/minecraft/recipe/brewing")).unwrap();
        std::fs::write(
            root.join("data/minecraft/recipe/brewing/test.json"),
            br#"{"type":"minecraft:brewing","output":{"id":"minecraft:emerald"}}"#,
        )
        .unwrap();
        let items = Items::for_tests(Default::default());
        let recipes = Brewing::load(&root, &items, &PackOverlay::NONE);
        assert_eq!(recipes.recipes[0].id, "example:brewing/test");
        assert_eq!(&*recipes.recipes[0].output.item, "minecraft:diamond");
        assert_eq!(recipes.recipes[1].id, "minecraft:brewing/test");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn content_brewing_recipe_condition_is_applied() {
        let root = content_brewing_root(
            "condition",
            r#", "neoforge:conditions":[{"type":"neoforge:not","value":{"type":"neoforge:true"}}]"#,
        );
        let items = Items::for_tests(Default::default());
        let recipes = Brewing::load(&root, &items, &PackOverlay::NONE);
        assert!(recipes.recipes.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn overlay_replaces_content_brewing_recipe() {
        let root = content_brewing_root("overlay", "");
        let files = [("recipe/example:brewing/test".to_owned(), vec![br#"{"type":"minecraft:brewing","input":{"item":"minecraft:potion"},"reagent":{"item":"minecraft:nether_wart"},"output":{"id":"minecraft:emerald"}}"#.to_vec()])];
        let items = Items::for_tests(Default::default());
        let recipes = Brewing::load(&root, &items, &PackOverlay::new(&files));
        assert_eq!(recipes.recipes.len(), 1);
        assert_eq!(&*recipes.recipes[0].output.item, "minecraft:emerald");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// `Ingredient.CODEC` (`HolderSetCodec`): a bare item id or `#tag` names
    /// `minecraft`'s, so `nether_wart` matches `minecraft:nether_wart` and
    /// `#planks` resolves the `minecraft:planks` tag.
    #[test]
    fn bare_ingredient_ids_name_minecrafts() {
        let mut tags = std::collections::HashMap::new();
        tags.insert(
            "minecraft:planks".to_owned(),
            ["minecraft:oak_planks".to_owned()]
                .into_iter()
                .collect::<HashSet<_>>(),
        );
        let items = Items::for_tests(tags);
        let item = PotionIngredient::parse(&serde_json::json!({"item": "nether_wart"}), &items);
        assert!(item.test(&ItemStack::new("minecraft:nether_wart", 1)));
        let tag = PotionIngredient::parse(&serde_json::json!({"item": "#planks"}), &items);
        assert!(tag.test(&ItemStack::new("minecraft:oak_planks", 1)));
        assert!(!tag.test(&ItemStack::new("minecraft:stone", 1)));
    }

    /// `ItemStackTemplate.CODEC` reads `output.id` via `Item.CODEC`: a bare id
    /// names `minecraft`'s.
    #[test]
    fn bare_output_id_names_minecrafts() {
        let root = content_brewing_root("bare-output", "");
        std::fs::create_dir_all(root.join("data/minecraft/recipe/brewing")).unwrap();
        std::fs::write(
            root.join("data/minecraft/recipe/brewing/bare.json"),
            br#"{"type":"minecraft:brewing","output":{"id":"emerald"}}"#,
        )
        .unwrap();
        let items = Items::for_tests(Default::default());
        let recipes = Brewing::load(&root, &items, &PackOverlay::NONE);
        let bare = recipes
            .recipes
            .iter()
            .find(|r| r.id == "minecraft:brewing/bare")
            .unwrap();
        assert_eq!(&*bare.output.item, "minecraft:emerald");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// `BrewingRecipe.output` is a whole `ItemStackTemplate`: a pack's
    /// custom name and potion contents beyond the potion id reach the brew.
    #[test]
    fn output_keeps_its_components() {
        let root = content_brewing_root("components", "");
        std::fs::write(
            root.join("data/example/recipe/brewing/named.json"),
            br#"{"type":"minecraft:brewing","input":{"item":"minecraft:potion"},"reagent":{"item":"minecraft:sugar"},"output":{"id":"minecraft:splash_potion","count":2,"components":{"minecraft:custom_name":"Zoom","minecraft:potion_contents":{"potion":"minecraft:swiftness","custom_color":16711680,"custom_effects":[{"id":"minecraft:speed","duration":600,"amplifier":2}]}}}}"#,
        )
        .unwrap();
        let items = Items::for_tests(Default::default());
        let recipes = Brewing::load(&root, &items, &PackOverlay::NONE);
        let out = &recipes
            .recipes
            .iter()
            .find(|r| r.id == "example:brewing/named")
            .unwrap()
            .output;
        assert_eq!(&*out.item, "minecraft:splash_potion");
        assert_eq!(out.count, 2);
        assert_eq!(out.custom_name().as_deref(), Some("Zoom"));
        assert_eq!(out.potion(), Some("minecraft:swiftness"));
        assert_eq!(
            out.custom_potion_effects(),
            vec![("minecraft:speed".to_owned(), 600, 2)]
        );
        let color = out
            .data()
            .and_then(|d| d.potion_extra.iter().find(|(k, _)| k == "custom_color"))
            .and_then(|(_, t)| horizon_client_core::nbt_value::number(Some(t)));
        assert_eq!(color, Some(16711680));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stack_fuel_controls_brew_duration_and_consumption() {
        let items = Items::for_tests(Default::default());
        let brewing = Brewing {
            reagents: ["minecraft:nether_wart".to_owned()].into_iter().collect(),
            inputs: HashSet::new(),
            recipes: vec![BrewingRecipe {
                id: "awkward".into(),
                input: PotionIngredient {
                    items: ["minecraft:potion".to_owned()].into_iter().collect(),
                    potions: Some(["minecraft:water".to_owned()].into_iter().collect()),
                },
                reagent: PotionIngredient {
                    items: ["minecraft:nether_wart".to_owned()].into_iter().collect(),
                    potions: None,
                },
                output: ItemStack::new("minecraft:potion", 1).with_potion("minecraft:awkward"),
            }],
        };
        use horizon_core::nbt::Tag;
        for uses in [Tag::Byte(7), Tag::Short(7), Tag::Int(7), Tag::Long(7)] {
            let mut fuel = ItemStack::new("minecraft:stone", 2);
            fuel.data_mut().set_loot_component(
                "minecraft:brewing_fuel",
                &Some(Tag::compound([
                    ("uses", uses),
                    ("speed_multiplier", Tag::Float(2.5_f32.to_bits())),
                ])),
            );
            let mut stand = BrewingStand::new();
            stand.items[0] =
                Some(ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water"));
            stand.items[3] = Some(ItemStack::new("minecraft:nether_wart", 1));
            stand.items[4] = Some(fuel);
            let providers = Providers::empty();
            stand.tick(&brewing, &items, &providers, &|s| data_remainder(&items, s));
            assert_eq!((stand.brew_time, stand.fuel, stand.total_fuel), (160, 6, 7));
            assert_eq!(stand.items[4].as_ref().unwrap().count, 1);
            for _ in 1..160 {
                assert!(
                    !stand
                        .tick(&brewing, &items, &providers, &|s| data_remainder(&items, s))
                        .brewed
                );
            }
            assert!(
                stand
                    .tick(&brewing, &items, &providers, &|s| data_remainder(&items, s))
                    .brewed
            );
            assert_eq!(
                stand.items[0].as_ref().unwrap().potion(),
                Some("minecraft:awkward")
            );
            assert_eq!(stand.fuel, 6);
            assert_eq!(stand.items[4].as_ref().unwrap().count, 1);
        }
    }

    #[test]
    fn potion_colours_and_names() {
        // Swiftness: speed colour 3402751 = 0x33EBFF.
        assert_eq!(potion_color(Some("minecraft:swiftness")), 0x33EBFF);
        assert_eq!(potion_color(Some("minecraft:water")), BASE_POTION_COLOR);
        // Turtle master mixes slowness x4 and resistance x3.
        let (s, r) = (9154528u32, 9520880u32);
        let mix = |sh: u32| ((s >> sh & 255) * 4 + (r >> sh & 255) * 3) / 7;
        assert_eq!(
            potion_color(Some("minecraft:turtle_master")),
            mix(16) << 16 | mix(8) << 8 | mix(0)
        );
        assert_eq!(
            potion_name_key("minecraft:splash_potion", Some("minecraft:long_swiftness")),
            "item.minecraft.splash_potion.effect.swiftness"
        );
        // A potion or a tipped arrow is named by its contents, over its
        // item_name; one whose contents were taken off is not.
        use horizon_core::nbt::Tag;
        let mut swift =
            ItemStack::new("minecraft:potion", 1).with_potion("minecraft:long_swiftness");
        swift
            .data_mut()
            .set_other("minecraft:item_name", Some(Tag::String("Bottle".into())));
        assert_eq!(
            contents_name_key(&swift).as_deref(),
            Some("item.minecraft.potion.effect.swiftness")
        );
        assert_eq!(
            contents_name_key(&ItemStack::new("minecraft:splash_potion", 1)).as_deref(),
            Some("item.minecraft.splash_potion.effect.empty")
        );
        let mut own = ItemStack::new("minecraft:tipped_arrow", 1).with_potion("minecraft:poison");
        own.data_mut()
            .potion_extra
            .push(("custom_name".into(), Tag::String("luck".into())));
        assert_eq!(
            contents_name_key(&own).as_deref(),
            Some("item.minecraft.tipped_arrow.effect.luck")
        );
        let mut taken = ItemStack::new("minecraft:potion", 1);
        taken.data_mut().set_other(
            "!minecraft:potion_contents",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        assert_eq!(contents_name_key(&taken), None);
        assert_eq!(
            contents_name_key(&ItemStack::new("minecraft:stone", 1)),
            None
        );
        assert_eq!(format_duration(9600), "08:00");
        assert_eq!(format_duration(450), "00:22");
        assert_eq!(format_duration(140), "00:07");
        assert_eq!(format_duration(1800), "01:30");
        assert_eq!(format_duration(72000), "01:00:00");
        assert_eq!(format_tick_duration(9600, 40.0), "04:00");
        let poison = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:strong_poison");
        let t = tooltip(&poison, 1.0, &|_| None);
        assert_eq!(t[0].0, "poison II (00:21)");
        assert_eq!(POTIONS.len(), 46);
    }

    #[test]
    fn brewing_stand_brews_after_400_ticks() {
        let mut item_tags = std::collections::HashMap::new();
        item_tags.insert("x".into(), HashSet::new());
        let items = Items::for_tests(item_tags);
        let recipe = BrewingRecipe {
            id: "awkward".into(),
            input: PotionIngredient {
                items: ["minecraft:potion".to_owned()].into_iter().collect(),
                potions: Some(["minecraft:water".to_owned()].into_iter().collect()),
            },
            reagent: PotionIngredient {
                items: ["minecraft:nether_wart".to_owned()].into_iter().collect(),
                potions: None,
            },
            output: ItemStack::new("minecraft:potion", 1).with_potion("minecraft:awkward"),
        };
        let brewing = Brewing {
            reagents: ["minecraft:nether_wart".to_owned()].into_iter().collect(),
            inputs: HashSet::new(),
            recipes: vec![recipe],
        };
        let providers = Providers::empty();
        let mut stand = BrewingStand::new();
        stand.fuel = 1;
        stand.items[0] = Some(ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water"));
        stand.items[2] =
            Some(ItemStack::new("minecraft:potion", 1).with_potion("minecraft:awkward"));
        stand.items[3] = Some(ItemStack::new("minecraft:nether_wart", 2));
        let first = stand.tick(&brewing, &items, &providers, &|s| data_remainder(&items, s));
        assert_eq!(first.bottles, Some([true, false, true]));
        assert_eq!((stand.brew_time, stand.fuel), (400, 0));
        let mut brewed_at = 0;
        for t in 1..=400 {
            if stand
                .tick(&brewing, &items, &providers, &|s| data_remainder(&items, s))
                .brewed
            {
                brewed_at = t;
            }
        }
        assert_eq!(brewed_at, 400);
        assert_eq!(
            stand.items[0].as_ref().unwrap().potion(),
            Some("minecraft:awkward")
        );
        // The awkward potion had no recipe and stays.
        assert_eq!(
            stand.items[2].as_ref().unwrap().potion(),
            Some("minecraft:awkward")
        );
        assert_eq!(stand.items[3].as_ref().unwrap().count, 1);
        // No fuel left: nothing more brews.
        stand.tick(&brewing, &items, &providers, &|s| data_remainder(&items, s));
        assert_eq!(stand.brew_time, 0);
    }

    #[test]
    fn a_brewing_stand_is_idle_only_while_its_tick_changes_nothing() {
        let mut stand = BrewingStand::new();
        assert!(!stand.idle());
        stand.last_bottles = Some([false; 3]);
        assert!(stand.idle());
        stand.items[0] = Some(ItemStack::new("minecraft:potion", 1));
        assert!(!stand.idle());
        stand.last_bottles = Some([true, false, false]);
        assert!(stand.idle());
        stand.items[4] = Some(ItemStack::new("minecraft:blaze_powder", 1));
        assert!(!stand.idle());
        stand.fuel = 3;
        assert!(stand.idle());
        stand.items[3] = Some(ItemStack::new("minecraft:nether_wart", 1));
        assert!(!stand.idle());
        stand.items[3] = None;
        stand.brew_time = 5;
        assert!(!stand.idle());
    }

    /// The stand sleeps while `idle`, so its tick must change nothing then: run the real
    /// tick on idle stands and compare, and check a stand with fuel to take wakes.
    #[test]
    fn an_idle_brewing_stand_tick_changes_nothing() {
        let items = Items::for_tests(std::collections::HashMap::new());
        let brewing = Brewing {
            reagents: ["minecraft:nether_wart".to_owned()].into_iter().collect(),
            inputs: HashSet::new(),
            recipes: vec![BrewingRecipe {
                id: "awkward".into(),
                input: PotionIngredient {
                    items: ["minecraft:potion".to_owned()].into_iter().collect(),
                    potions: Some(["minecraft:water".to_owned()].into_iter().collect()),
                },
                reagent: PotionIngredient {
                    items: ["minecraft:nether_wart".to_owned()].into_iter().collect(),
                    potions: None,
                },
                output: ItemStack::new("minecraft:potion", 1).with_potion("minecraft:awkward"),
            }],
        };
        let providers = Providers::empty();
        let water = || Some(ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water"));
        let stand = |bottle: bool, fuel: i32, powder: bool| {
            let mut s = BrewingStand::new();
            s.fuel = fuel;
            if bottle {
                s.items[1] = water();
            }
            if powder {
                s.items[4] = Some(ItemStack::new("minecraft:blaze_powder", 1));
            }
            s.last_bottles = Some([false, bottle, false]);
            s
        };
        for before in [
            stand(false, 0, false),
            stand(true, 0, false),
            stand(true, 5, false),
            stand(true, 5, true),
        ] {
            assert!(before.idle(), "{before:?}");
            let mut after = before.clone();
            assert_eq!(
                after.tick(&brewing, &items, &providers, &|s| data_remainder(&items, s)),
                BrewTick::default()
            );
            assert_eq!(format!("{after:?}"), format!("{before:?}"));
        }
        // A brewable ingredient starts the brew: not idle, and the tick changes it.
        let mut brewing_now = stand(true, 5, false);
        brewing_now.items[3] = Some(ItemStack::new("minecraft:nether_wart", 1));
        assert!(!brewing_now.idle());
        brewing_now.tick(&brewing, &items, &providers, &|s| data_remainder(&items, s));
        assert_eq!(brewing_now.brew_time, 400);
    }

    /// #3312: `PotionContents.getPotionDescription` names the effects with
    /// translatable components, so a Russian game reads them from ru_ru.json
    /// (the prepared one; skipped without it), as the tooltip's other lines.
    #[test]
    fn effect_lines_read_the_selected_language() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("assets/minecraft/lang/ru_ru.json").exists() {
            return;
        }
        let ru = crate::lang::Language::load(&[&root], "ru_ru");
        let translate = |key: &str| ru.get(key).map(str::to_owned);
        let master =
            ItemStack::new("minecraft:tipped_arrow", 1).with_potion("minecraft:turtle_master");
        let lines = tooltip(&master, 0.125, &translate);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].0.starts_with("Замедление IV ("), "{lines:?}");
        assert!(lines[1].0.starts_with("Сопротивление III ("), "{lines:?}");
    }

    /// Cancelled pre-hook slots replace the brew without consuming an ingredient.
    /// Uncancelled pre-hook edits are discarded before vanilla brewing runs.
    #[test]
    fn a_cancelled_brew_takes_the_mods_slots_and_an_uncancelled_one_brews() {
        let set = |ids: &[&str]| ids.iter().map(|s| (*s).to_owned()).collect();
        let brewing = Brewing {
            reagents: set(&["minecraft:nether_wart"]),
            inputs: Default::default(),
            recipes: vec![BrewingRecipe {
                id: "awkward".into(),
                input: PotionIngredient {
                    items: set(&["minecraft:potion"]),
                    potions: Some(set(&["minecraft:water"])),
                },
                reagent: PotionIngredient {
                    items: set(&["minecraft:nether_wart"]),
                    potions: None,
                },
                output: ItemStack::new("minecraft:potion", 1).with_potion("minecraft:awkward"),
            }],
        };
        let items = horizon_items::items::Items::for_tests(Default::default());
        let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
        let awkward = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:awkward");
        let fresh = || {
            let mut stand = BrewingStand::new();
            stand.items[0] = Some(water.clone());
            stand.items[3] = Some(ItemStack::new("minecraft:nether_wart", 2));
            stand
        };
        for cancel in [true, false] {
            let mut stand = fresh();
            let mut edited = stand.items.clone();
            edited[0] = Some(ItemStack::new("minecraft:diamond", 1));
            let mut out = BrewTick::default();
            stand.brew_after_pre(
                Some((edited, cancel)),
                &brewing,
                &|s| data_remainder(&items, s),
                &mut out,
            );
            assert_eq!(out.brewed, !cancel);
            assert_eq!(
                stand.items[0],
                Some(if cancel {
                    ItemStack::new("minecraft:diamond", 1)
                } else {
                    awkward.clone()
                })
            );
            assert!(stand.items[1].is_none());
            assert_eq!(
                stand.items[3].as_ref().map(|s| s.count),
                Some(if cancel { 2 } else { 1 })
            );
        }
    }

    #[test]
    fn infinite_effects_say_the_infinite_sign() {
        // MobEffectInstance.isInfiniteDuration: a -1 duration says the
        // infinite sign, not a time; a short one still says nothing.
        let translate = |key: &str| (key == "effect.duration.infinite").then(|| "∞".to_owned());
        let lines = effect_lines([("minecraft:resistance", -1, 0)], 1.0, &translate);
        assert_eq!(lines[0].0, "resistance (∞)");
        let lines = effect_lines([("minecraft:resistance", 15, 0)], 1.0, &translate);
        assert_eq!(lines[0].0, "resistance");
    }
    /// `PotionContents.addPotionTooltip` lists `getAllEffects`: the potion's
    /// effects, then the custom ones, and a potion with only custom effects
    /// is no "No Effects".
    #[test]
    fn the_tooltip_lists_custom_effects_too() {
        use horizon_core::nbt::Tag;
        let custom = |id: &str, duration: i32, amplifier: i32| {
            Tag::compound([
                ("id", Tag::String(id.into())),
                ("duration", Tag::Int(duration)),
                ("amplifier", Tag::Int(amplifier)),
            ])
        };
        let mut stack = ItemStack::new("minecraft:potion", 1);
        assert!(tooltip(&stack, 1.0, &|_| None).is_empty());
        stack.data_mut().potion_extra = vec![(
            "custom_effects".into(),
            Tag::List(vec![custom("minecraft:speed", 600, 1)]),
        )];
        let lines: Vec<_> = tooltip(&stack, 1.0, &|_| None)
            .into_iter()
            .map(|l| l.0)
            .collect();
        assert_eq!(lines, ["speed II (00:30)"]);
        let stack = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:strong_poison");
        let mut mixed = stack.clone();
        mixed.data_mut().potion_extra = vec![(
            "custom_effects".into(),
            Tag::List(vec![custom("minecraft:glowing", 200, 0)]),
        )];
        let lines: Vec<_> = tooltip(&mixed, 1.0, &|_| None)
            .into_iter()
            .map(|l| l.0)
            .collect();
        assert_eq!(lines, ["poison II (00:21)", "glowing (00:10)"]);
    }
    #[test]
    fn custom_contents_color_and_empty_semantics() {
        use horizon_core::nbt::Tag;
        let mut stack = ItemStack::new("minecraft:tipped_arrow", 1);
        assert!(contents_empty(&stack));
        stack
            .data_mut()
            .potion_extra
            .push(("custom_effects".into(), Tag::List(Vec::new())));
        assert!(contents_empty(&stack));
        stack.data_mut().potion_extra = vec![(
            "custom_effects".into(),
            Tag::List(vec![
                Tag::compound([
                    ("id", Tag::String("minecraft:glowing".into())),
                    ("amplifier", Tag::Int(1)),
                ]),
                Tag::compound([
                    ("id", Tag::String("minecraft:poison".into())),
                    ("show_particles", Tag::Byte(0)),
                ]),
            ]),
        )];
        assert!(!contents_empty(&stack));
        // Glowing's colour; hidden poison contributes no weight.
        assert_eq!(contents_color(&stack), 0xFF94_A061);
        stack
            .data_mut()
            .potion_extra
            .push(("custom_color".into(), Tag::Int(123456)));
        assert_eq!(contents_color(&stack), 123456);
        stack.data_mut().potion_extra = vec![("custom_name".into(), Tag::String("test".into()))];
        assert!(!contents_empty(&stack));
        assert_eq!(contents_color(&stack), 0xFF38_5DC6);
    }
}
