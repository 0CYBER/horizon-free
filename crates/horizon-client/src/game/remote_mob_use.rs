//! A server's client right-clicking a mob: vanilla's client runs
//! `Player.interactOn` on its own level (`MultiPlayerGameMode.interact`) to
//! learn whether the click ends on the mob or goes on to the held item
//! (`Minecraft.startUseItem` then sends UseItem). The server does the real
//! interaction; this is what the client's copy of the mob answers.
use super::Game;
use horizon_client::{
    first_person::Hand,
    mobs::{Species, equine, llama},
};

impl Game<'_> {
    /// Whether mob `i` takes a right click with `hand`'s item on this
    /// client (`Mob.interact`'s result consumes the action).
    pub(super) fn remote_mob_takes(&self, i: usize, hand: Hand) -> bool {
        let Some(m) = self.mobs.get(i).filter(|m| m.alive()) else {
            return false;
        };
        let stack = match hand {
            Hand::Main => self.stack_in(self.selected),
            Hand::Off => self.offhand.as_ref(),
        };
        let item = stack.map_or("", |s| &*s.item);
        let baby = m.is_baby();
        // Mob.checkAndHandleImportantInteractions: a lead, a named name tag,
        // a spawn egg of the mob's own kind.
        if item == "minecraft:lead"
            || item == "minecraft:name_tag" && stack.is_some_and(|s| s.custom_name().is_some())
            || item.strip_suffix("_spawn_egg") == Some(m.kind.id.as_str())
        {
            return true;
        }
        // Animal.mobInteract: its food is CONSUME on a client.
        if m.kind.flags.animal && m.kind.food.iter().any(|f| f.as_str() == item) {
            return true;
        }
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        match m.species() {
            // Villager / WanderingTrader.mobInteract: trading, or a baby's head shake.
            Species::Villager | Species::WanderingTrader => item != "minecraft:villager_spawn_egg",
            // AbstractHorse, Camel: ridden (or their inventory opened); a baby only eats.
            s if equine::is_equine(s)
                || llama::is_llama(s)
                || matches!(s, Species::Camel | Species::CamelHusk) =>
            {
                !baby
            }
            // Pig / Strider.mobInteract: a saddled one is ridden.
            Species::Pig | Species::Strider => m.extra.saddled && !sneaking,
            // HappyGhast.mobInteract: a harnessed adult is ridden.
            Species::HappyGhast => !baby && m.extra.head.is_some(),
            // TamableAnimal (wolf, cat, parrot) on a client: CONSUME once tame.
            Species::Wolf | Species::Cat | Species::Parrot => m.extra.tame,
            // Allay.mobInteract: gives an item or takes it back.
            Species::Allay => !item.is_empty() || m.extra.main_hand.is_some(),
            Species::Cow | Species::Goat => item == "minecraft:bucket" && !baby,
            Species::Mooshroom => {
                matches!(
                    item,
                    "minecraft:bucket" | "minecraft:bowl" | "minecraft:shears"
                ) && !baby
            }
            Species::Sheep => item == "minecraft:shears" || item.ends_with("_dye"),
            // Bucketable.bucketMobPickup.
            Species::Cod
            | Species::Salmon
            | Species::TropicalFish
            | Species::Pufferfish
            | Species::Axolotl
            | Species::Tadpole => item == "minecraft:water_bucket",
            Species::Armadillo => item == "minecraft:brush",
            // ZombieVillager.mobInteract: CONSUME for a golden apple.
            Species::ZombieVillager => item == "minecraft:golden_apple",
            // Creeper.mobInteract: CREEPER_IGNITERS.
            Species::Creeper => {
                matches!(item, "minecraft:flint_and_steel" | "minecraft:fire_charge")
            }
            // PiglinAi.mobInteract: an adult admires what it loves.
            Species::Piglin => {
                !baby
                    && self
                        .res
                        .items()
                        .item_tags
                        .get("minecraft:piglin_loved")
                        .is_some_and(|tag| tag.contains(item))
            }
            _ => false,
        }
    }
}
