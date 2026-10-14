//! Survival rules of the world generator's feature placement: each block's rule from the class
//! vanilla gives it and the class of its placement check (tools/test_block_survival_parity.py's
//! `bind_definitions` and its tables).

use crate::pyjson::Value;

const CLASS_RULES: &[(&str, &str)] = &[
    ("FireBlock", "Fire"),
    ("CactusBlock", "Cactus"),
    ("PathBlock", "Path"),
    ("MushroomBlock", "Mushroom"),
    ("MultifaceBlock", "Multiface"),
    ("VineBlock", "Vine"),
    ("HangingMossBlock", "HangingMoss"),
    ("SeaPickleBlock", "SeaPickle"),
    ("SnowLayerBlock", "SnowLayer"),
    ("BellBlock", "Bell"),
    ("ShelfMushroomBlock", "FullBehind"),
    ("GrowingPlantBlock", "GrowingPlant"),
    ("MangrovePropaguleBlock", "MangrovePropagule"),
    ("SpeleothemBlock", "Speleothem"),
    ("ScaffoldingBlock", "Scaffolding"),
    ("PistonHeadBlock", "PistonHead"),
    ("ChorusPlantBlock", "ChorusPlant"),
    ("ChorusFlowerBlock", "ChorusFlower"),
    ("PitcherCropBlock", "PitcherCrop"),
    ("BigDripleafBlock", "BigDripleaf"),
    ("BigDripleafStemBlock", "BigDripleafStem"),
    ("FarmlandBlock", "Farmland"),
    ("CocoaBlock", "Cocoa"),
    ("SugarCaneBlock", "SugarCane"),
    ("SporeBlossomBlock", "SporeBlossom"),
    ("FrogspawnBlock", "Frogspawn"),
    ("SmallDripleafBlock", "SmallDripleaf"),
    ("TallSeagrassBlock", "TallSeagrass"),
    ("DoorBlock", "Door"),
    ("BaseRailBlock", "RigidBelow"),
    ("LeafLitterBlock", "FullBelow"),
    ("WallSignBlock", "SolidBehind"),
    ("GrindstoneBlock", "Always"),
    ("RedstoneWallTorchBlock", "FullBehind"),
    ("BubbleColumnBlock", "BubbleColumn"),
    ("RedstoneWireBlock", "RedstoneWire"),
    ("SoulFireBlock", "TagBelow:minecraft:soul_fire_base_blocks"),
    ("BlockBehaviour", "Always"),
    ("CakeBlock", "SolidBelow"),
    ("CandleCakeBlock", "SolidBelow"),
    ("BannerBlock", "SolidBelow"),
    ("StandingSignBlock", "SolidBelow"),
    ("WallBannerBlock", "SolidBehind"),
    ("CarpetBlock", "NonAirBelow"),
    ("MossyCarpetBlock", "MossCarpet"),
    ("BaseTorchBlock", "CenterBelow"),
    ("CandleBlock", "CenterBelow"),
    ("LanternBlock", "Lantern"),
    ("HangingRootsBlock", "FullAbove"),
    ("LadderBlock", "FullBehind"),
    ("AmethystClusterBlock", "FullBehind"),
    ("TripWireHookBlock", "FullBehind"),
    ("BaseCoralWallFanBlock", "FullBehind"),
    ("WallTorchBlock", "FullBehind"),
    ("BasePressurePlateBlock", "RigidOrCenterBelow"),
    ("DiodeBlock", "RigidBelow"),
    ("BaseCoralPlantTypeBlock", "FullBelow"),
    ("CeilingHangingSignBlock", "CenterAbove"),
    ("FaceAttachedHorizontalDirectionalBlock", "FaceAttached"),
    ("BambooSaplingBlock", "SupportsBamboo"),
    ("BambooStalkBlock", "SupportsBamboo"),
];

const VEGETATION_RULES: &[(&str, &str)] = &[
    ("LilyPadBlock", "LilyPad"),
    ("VegetationBlock", "TagBelow:minecraft:supports_vegetation"),
    ("AzaleaBlock", "TagBelow:minecraft:supports_azalea"),
    (
        "DryVegetationBlock",
        "TagBelow:minecraft:supports_dry_vegetation",
    ),
    (
        "NetherSproutsBlock",
        "TagBelow:minecraft:supports_nether_sprouts",
    ),
    ("NetherWartBlock", "TagBelow:minecraft:supports_nether_wart"),
    ("WitherRoseBlock", "TagBelow:minecraft:supports_wither_rose"),
    ("CactusFlowerBlock", "CactusFlower"),
    ("SeagrassBlock", "Seagrass"),
];

fn lookup(table: &[(&str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// `{block: rule}` from the exporter's `{block: class}`, `{block: placement class}` and
/// `{block: {field: support tag}}`, in the order Python's dict built it.
pub fn bind_definitions(
    classes: &Value,
    placement: &Value,
    support_tags: &Value,
) -> Result<Value, String> {
    let classes = classes
        .as_object()
        .ok_or("survival-classes.json is not an object")?;
    let mut definitions = Value::Object(Vec::new());
    for (name, owner) in classes {
        if let Some(rule) = owner.as_str().and_then(|o| lookup(CLASS_RULES, o)) {
            definitions.insert(name, Value::str(rule));
        }
    }
    for (name, owner) in classes {
        let owner = owner.as_str().unwrap_or("");
        let placed = placement.get(name).and_then(Value::as_str);
        if owner == "CropBlock" && placed == Some("CropBlock") {
            definitions.insert(name, Value::str("Crop"));
        }
        if owner == "DoublePlantBlock" && placed == Some("VegetationBlock") {
            definitions.insert(name, Value::str("DoublePlant"));
        }
        if owner != "VegetationBlock" {
            continue;
        }
        let method = placed.ok_or_else(|| format!("No placement class for {name}"))?;
        if let Some(rule) = lookup(VEGETATION_RULES, method) {
            definitions.insert(name, Value::str(rule));
        } else if [
            "AttachedStemBlock",
            "StemBlock",
            "NetherFungusBlock",
            "NetherRootsBlock",
        ]
        .contains(&method)
        {
            let field = if method == "StemBlock" {
                "stemSupportBlocks"
            } else {
                "supportBlocks"
            };
            let tag = support_tags
                .get(name)
                .and_then(|t| t.get(field))
                .and_then(Value::as_str)
                .ok_or_else(|| format!("No {field} support tag for {name}"))?;
            definitions.insert(name, Value::Str(format!("TagBelow:{tag}")));
        }
    }
    Ok(definitions)
}
