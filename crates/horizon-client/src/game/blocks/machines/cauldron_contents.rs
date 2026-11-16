//! What a cauldron block holds for the fluid capability (`FLUIDS`): one table of contents, as
//! NeoForge's `CauldronFluidContent`. Vanilla's water and lava cauldrons are built in; the
//! ones mods register through `register_cauldron_fluid_content` join them once the block
//! registry is resolved (`mods::register_cauldron_fluid_contents`). `native_fluids` reads it.
use horizon_world::registry::BlockRegistry;
use std::sync::{Arc, LazyLock, RwLock};

/// The empty cauldron: what a drained content leaves, and what a fill of any content takes.
pub(in crate::game) const EMPTY_CAULDRON: &str = "minecraft:cauldron";

/// The same vanilla content definitions seed runtime tables and registration coverage.
const VANILLA_CONTENTS: [(&str, &str, Option<(&str, u32)>); 2] = [
    (
        "minecraft:water_cauldron",
        "minecraft:water",
        Some(("level", 3)),
    ),
    ("minecraft:lava_cauldron", "minecraft:lava", None),
];

/// Type coverage only: the empty cauldron or a vanilla content's block.
pub(in crate::game) fn vanilla_block_has_fluids(block: &str) -> bool {
    block == EMPTY_CAULDRON || VANILLA_CONTENTS.iter().any(|&(name, _, _)| name == block)
}

/// A cauldron block's content: `total` millibuckets of `fluid` when full, over the levels of
/// the block's integer property `level`, or only ever full when it has none (lava).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::game) struct CauldronContent {
    /// The block's registry name.
    pub block: String,
    /// The fluid's registry name.
    pub fluid: String,
    /// Millibuckets when full.
    pub total: u32,
    /// The block's integer level property and its greatest value, the number of levels.
    pub level: Option<(String, u32)>,
}

impl CauldronContent {
    /// A mod's registration resolved against `registry`: `level`, when named, must be an
    /// integer property of `block`, whose greatest value is the number of levels, and `total`
    /// must be positive and divide evenly by them. Refused with the reason, naming what is
    /// wrong.
    pub fn resolve(
        registry: &BlockRegistry,
        block: &str,
        fluid: &str,
        total: u32,
        level: Option<&str>,
    ) -> Result<Self, String> {
        let level = match level {
            None => None,
            Some(property) => {
                let values = registry.property_values(block, property);
                if values.is_empty() {
                    return Err(format!(
                        "{block} has no property {property} to hold {fluid}'s level"
                    ));
                }
                let mut integers = values
                    .iter()
                    .map(|value| {
                        value
                            .parse::<u32>()
                            .ok()
                            .filter(|n| n.to_string() == *value)
                            .ok_or(())
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| {
                        format!("{block}'s property {property} isn't an integer property")
                    })?;
                integers.sort_unstable();
                let levels = *integers.last().unwrap();
                let first = integers[0];
                if levels == 0
                    || first > 1
                    || integers
                        .windows(2)
                        .any(|pair| pair[0].checked_add(1) != Some(pair[1]))
                {
                    return Err(format!(
                        "{block}'s property {property} must contain every integer level from 1"
                    ));
                }
                Some((property.to_owned(), levels))
            }
        };
        let levels = level.as_ref().map_or(1, |&(_, levels)| levels);
        if total > i32::MAX as u32 {
            return Err(format!(
                "{block} holds more {fluid} than the fluid capability can represent"
            ));
        }
        if total == 0 {
            return Err(format!("{block} would hold 0 mB of {fluid}"));
        }
        if !total.is_multiple_of(levels) {
            return Err(format!(
                "{block} would hold {total} mB of {fluid}, which doesn't divide evenly by its \
                 {levels} levels"
            ));
        }
        Ok(Self {
            block: block.to_owned(),
            fluid: fluid.to_owned(),
            total,
            level,
        })
    }
    /// How many levels fill it: one for a cauldron only ever full.
    pub fn levels(&self) -> u32 {
        self.level.as_ref().map_or(1, |&(_, levels)| levels)
    }
    /// What `level` levels hold: `level * total / levels`, so each level holds its share of
    /// `total`, and water keeps `LayeredCauldronBlock`'s thirds of a bucket (333, 666, 1000),
    /// which has no finer unit than a bottle.
    pub fn amount(&self, level: u32) -> i32 {
        let held = u64::from(level) * u64::from(self.total) / u64::from(self.levels());
        i32::try_from(held).unwrap_or(i32::MAX)
    }
    /// The level a fill of `given` millibuckets raises `level` to: a level rises only while
    /// its whole amount fits what is left of the give, so no sum ever grows past `total`.
    pub fn fill_level(&self, level: u32, given: i32) -> u32 {
        let mut new = level;
        while new < self.levels() && self.amount(new + 1) - self.amount(level) <= given {
            new += 1;
        }
        new
    }
    /// What a drain of at most `amount` takes out of level `level`, in whole levels — never
    /// more than was asked, as `IFluidHandler.drain`'s `maxDrain`: less than the topmost
    /// whole level takes nothing. Answers what it took and the level it left.
    pub fn drain_take(&self, level: u32, amount: i32) -> (i32, u32) {
        let mut taken = 0;
        let mut left = level;
        while left > 0 {
            let step = self.amount(left) - self.amount(left - 1);
            if taken + step > amount {
                break;
            }
            taken += step;
            left -= 1;
        }
        (taken, left)
    }
}

/// Every cauldron content: one per block, and one per fluid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::game) struct CauldronContents {
    contents: Vec<CauldronContent>,
}

impl CauldronContents {
    /// Vanilla's: water, a bucket in thirds over `level` 1..3, and lava, a bucket only whole.
    pub fn vanilla() -> Self {
        Self {
            contents: VANILLA_CONTENTS
                .iter()
                .map(|&(block, fluid, level)| CauldronContent {
                    block: block.to_owned(),
                    fluid: fluid.to_owned(),
                    total: 1000,
                    level: level.map(|(property, levels)| (property.to_owned(), levels)),
                })
                .collect(),
        }
    }
    /// Adds `content`. Refused, naming both sides, when its block already has a content or
    /// its fluid already has a cauldron: one content per block and per fluid.
    pub fn add(&mut self, content: CauldronContent) -> Result<(), String> {
        if let Some((_, held)) = self.of_block(&content.block) {
            return Err(format!(
                "{} already holds {}, so it can't hold {}",
                content.block, held.fluid, content.fluid
            ));
        }
        if let Some((_, held)) = self.of_fluid(&content.fluid) {
            return Err(format!(
                "{} already has a cauldron, {}, so {} can't be one",
                content.fluid, held.block, content.block
            ));
        }
        self.contents.push(content);
        Ok(())
    }
    /// The content of block `block`, with its index.
    pub fn of_block(&self, block: &str) -> Option<(usize, &CauldronContent)> {
        self.contents
            .iter()
            .enumerate()
            .find(|(_, content)| content.block == block)
    }
    /// The content holding fluid `fluid`, with its index.
    pub fn of_fluid(&self, fluid: &str) -> Option<(usize, &CauldronContent)> {
        self.contents
            .iter()
            .enumerate()
            .find(|(_, content)| content.fluid == fluid)
    }
    /// The content at `index`, from `of_block` or `of_fluid`.
    pub fn get(&self, index: usize) -> &CauldronContent {
        &self.contents[index]
    }
}

/// The table in place: vanilla's contents until a resource load installs the mods' with them.
static CONTENTS: LazyLock<RwLock<Arc<CauldronContents>>> =
    LazyLock::new(|| RwLock::new(Arc::new(CauldronContents::vanilla())));

static BUILT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The published table, absent until resource registration has built it.
pub(in crate::game) fn current_if_built() -> Option<Arc<CauldronContents>> {
    BUILT
        .load(std::sync::atomic::Ordering::Acquire)
        .then(current)
}

#[cfg(test)]
pub(in crate::game) fn without_built_table(test: impl FnOnce()) {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            BUILT.store(self.0, std::sync::atomic::Ordering::Release);
        }
    }
    let _restore = Restore(BUILT.swap(false, std::sync::atomic::Ordering::AcqRel));
    test();
}

/// The table in place.
pub(in crate::game) fn current() -> Arc<CauldronContents> {
    CONTENTS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Puts `contents`, a resource load's whole table, in place of the one there.
pub(in crate::game) fn install(contents: CauldronContents) {
    *CONTENTS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(contents);
    BUILT.store(true, std::sync::atomic::Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_type_coverage_matches_the_runtime_content_definitions() {
        let contents = CauldronContents::vanilla();
        for block in [
            "minecraft:cauldron",
            "minecraft:water_cauldron",
            "minecraft:lava_cauldron",
        ] {
            assert!(vanilla_block_has_fluids(block), "{block}");
            assert!(block == EMPTY_CAULDRON || contents.of_block(block).is_some());
        }
        for block in [
            "minecraft:powder_snow_cauldron",
            "minecraft:chest",
            "test:water_cauldron",
        ] {
            assert!(!vanilla_block_has_fluids(block), "{block}");
            assert!(contents.of_block(block).is_none());
        }
    }

    fn water() -> CauldronContent {
        CauldronContents::vanilla()
            .of_block("minecraft:water_cauldron")
            .unwrap()
            .1
            .clone()
    }

    fn syrup(total: u32, levels: Option<u32>) -> CauldronContent {
        CauldronContent {
            block: "test:syrup_cauldron".to_owned(),
            fluid: "test:syrup".to_owned(),
            total,
            level: levels.map(|levels| ("level".to_owned(), levels)),
        }
    }

    #[test]
    fn a_cauldron_level_holds_a_third_of_a_bucket() {
        let water = water();
        assert_eq!(water.amount(1), 333);
        assert_eq!(water.amount(2), 666);
        assert_eq!(water.amount(3), 1000);
        // A mod's content holds total ÷ levels per level, and the whole when it has no level.
        let thirds = syrup(3000, Some(3));
        assert_eq!(thirds.amount(1), 1000);
        assert_eq!(thirds.amount(2), 2000);
        assert_eq!(thirds.amount(3), 3000);
        let whole = syrup(4000, None);
        assert_eq!(whole.levels(), 1);
        assert_eq!(whole.amount(1), 4000);
    }

    #[test]
    fn a_cauldron_never_hands_out_more_than_was_asked() {
        let water = water();
        // The top third of a full cauldron is 334, then 333, 333.
        assert_eq!(water.drain_take(3, 1), (0, 3));
        assert_eq!(water.drain_take(3, 333), (0, 3));
        assert_eq!(water.drain_take(3, 334), (334, 2));
        assert_eq!(water.drain_take(3, 500), (334, 2));
        assert_eq!(water.drain_take(3, 666), (334, 2));
        assert_eq!(water.drain_take(3, 667), (667, 1));
        assert_eq!(water.drain_take(3, 1000), (1000, 0));
        assert_eq!(water.drain_take(1, 332), (0, 1));
        assert_eq!(water.drain_take(1, 333), (333, 0));
        assert_eq!(water.drain_take(1, i32::MAX), (333, 0));
        let syrup = syrup(3000, Some(3));
        assert_eq!(syrup.drain_take(3, 999), (0, 3));
        assert_eq!(syrup.drain_take(3, 2500), (2000, 1));
        assert_eq!(syrup.drain_take(2, i32::MAX), (2000, 0));
    }

    #[test]
    fn a_cauldron_fill_stays_within_a_bucket_whatever_the_amount() {
        let water = water();
        assert_eq!(water.fill_level(0, 332), 0);
        assert_eq!(water.fill_level(0, 333), 1);
        assert_eq!(water.fill_level(1, 332), 1);
        assert_eq!(water.fill_level(1, 333), 2);
        assert_eq!(water.fill_level(1, 666), 2);
        assert_eq!(water.fill_level(1, 667), 3);
        assert_eq!(water.fill_level(0, i32::MAX), 3);
        assert_eq!(water.fill_level(2, i32::MAX), 3);
        let thirds = syrup(3000, Some(3));
        assert_eq!(thirds.fill_level(0, 999), 0);
        assert_eq!(thirds.fill_level(0, 2500), 2);
        assert_eq!(thirds.fill_level(1, i32::MAX), 3);
        let whole = syrup(4000, None);
        assert_eq!(whole.fill_level(0, 3999), 0);
        assert_eq!(whole.fill_level(0, 4000), 1);
        assert_eq!(whole.fill_level(1, i32::MAX), 1);
    }

    #[test]
    fn one_content_per_block_and_per_fluid_each_refusal_naming_both_sides() {
        let mut contents = CauldronContents::vanilla();
        let honey = |block: &str| CauldronContent {
            block: block.to_owned(),
            fluid: "test:honey".to_owned(),
            total: 1000,
            level: None,
        };
        let taken_block = contents.add(honey("minecraft:water_cauldron")).unwrap_err();
        assert!(
            taken_block.contains("minecraft:water_cauldron")
                && taken_block.contains("minecraft:water")
                && taken_block.contains("test:honey"),
            "{taken_block}"
        );
        let taken_fluid = contents
            .add(CauldronContent {
                fluid: "minecraft:water".to_owned(),
                ..honey("test:basin")
            })
            .unwrap_err();
        assert!(
            taken_fluid.contains("minecraft:water")
                && taken_fluid.contains("minecraft:water_cauldron")
                && taken_fluid.contains("test:basin"),
            "{taken_fluid}"
        );
        // The refusals keep nothing; the next content still applies.
        contents.add(honey("test:basin")).unwrap();
        assert_eq!(
            contents.of_block("test:basin").map(|(index, _)| index),
            Some(2)
        );
        assert_eq!(
            contents.of_fluid("test:honey").map(|(index, _)| index),
            Some(2)
        );
        assert_eq!(contents.get(2).fluid, "test:honey");
        assert!(contents.of_block("minecraft:water").is_none());
        assert_eq!(
            contents
                .of_fluid("minecraft:lava")
                .map(|(_, content)| content.block.as_str()),
            Some("minecraft:lava_cauldron")
        );
    }

    #[test]
    fn registration_refuses_sparse_and_mixed_levels_but_accepts_large_ranges() {
        for (values, accepted) in [
            (vec!["1".to_owned(), "3".to_owned()], false),
            (vec!["1".to_owned(), "bad".to_owned()], false),
            ((1..=300).map(|n| n.to_string()).collect(), true),
        ] {
            let states: Vec<_> = values.iter().map(|value| serde_json::json!(
                ["p", false, false, "", {"id":"test:cauldron", "properties":{"level":value}}]
            )).collect();
            let facts = serde_json::json!({"defaults":{"test:cauldron":0},"states":states});
            let registry = BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap();
            assert_eq!(
                CauldronContent::resolve(
                    &registry,
                    "test:cauldron",
                    "test:syrup",
                    3000,
                    Some("level")
                )
                .is_ok(),
                accepted
            );
        }
    }

    #[test]
    fn a_registration_resolves_its_level_property_and_total_against_the_registry() {
        let facts = r#"{"defaults":{"minecraft:cauldron":0,"minecraft:powder_snow_cauldron":1,"minecraft:stone":4,"minecraft:hay_block":5},"states":[
            ["c",false,false,"","minecraft:cauldron"],
            ["p",false,false,"",{"id":"minecraft:powder_snow_cauldron","properties":{"level":"1"}}],
            ["p",false,false,"",{"id":"minecraft:powder_snow_cauldron","properties":{"level":"2"}}],
            ["p",false,false,"",{"id":"minecraft:powder_snow_cauldron","properties":{"level":"3"}}],
            ["s",false,false,"","minecraft:stone"],
            ["h",false,false,"",{"id":"minecraft:hay_block","properties":{"axis":"x"}}],
            ["h",false,false,"",{"id":"minecraft:hay_block","properties":{"axis":"y"}}]
        ]}"#;
        let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        let resolve = |block: &str, total: u32, level: Option<&str>| {
            CauldronContent::resolve(&registry, block, "test:syrup", total, level)
        };
        assert_eq!(
            resolve("minecraft:powder_snow_cauldron", 3000, Some("level")),
            Ok(CauldronContent {
                block: "minecraft:powder_snow_cauldron".to_owned(),
                fluid: "test:syrup".to_owned(),
                total: 3000,
                level: Some(("level".to_owned(), 3)),
            })
        );
        assert_eq!(
            resolve("minecraft:stone", 4000, None),
            Ok(CauldronContent {
                block: "minecraft:stone".to_owned(),
                fluid: "test:syrup".to_owned(),
                total: 4000,
                level: None,
            })
        );
        assert!(resolve("minecraft:powder_snow_cauldron", u32::MAX, Some("level")).is_err());
        let missing = resolve("minecraft:stone", 1000, Some("level")).unwrap_err();
        assert!(
            missing.contains("minecraft:stone") && missing.contains("level"),
            "{missing}"
        );
        let not_integer = resolve("minecraft:hay_block", 1000, Some("axis")).unwrap_err();
        assert!(
            not_integer.contains("minecraft:hay_block")
                && not_integer.contains("axis")
                && not_integer.contains("integer"),
            "{not_integer}"
        );
        let nothing = resolve("minecraft:powder_snow_cauldron", 0, Some("level")).unwrap_err();
        assert!(nothing.contains("0 mB"), "{nothing}");
        let uneven = resolve("minecraft:powder_snow_cauldron", 1000, Some("level")).unwrap_err();
        assert!(
            uneven.contains("1000 mB") && uneven.contains("3 levels"),
            "{uneven}"
        );
    }
}
