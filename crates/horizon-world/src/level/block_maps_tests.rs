//! The world's NeoForge block maps on fire, weathering, lightning, waxing and the axe, each
//! against vanilla's answer with no map, and vanilla's answers again once a reload installs none.
use super::*;
use crate::level::BlockMaps;
use crate::level::tests::{World, fixture};
use serde_json::json;
use std::sync::Arc;

/// Takes the maps off the shared fixture's behaviours however the test ends.
struct Uninstall<'b>(&'b Behaviours);
impl Drop for Uninstall<'_> {
    fn drop(&mut self) {
        self.0.set_block_maps(None);
    }
}

#[test]
fn the_block_maps_drive_fire_weathering_waxing_and_scraping_until_a_reload_removes_them() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f);
    let b = w.b();
    let _uninstall = Uninstall(b);
    // Blocks no other level test burns, weathers or waxes: the fixture's behaviours are shared.
    let end_stone = w.id("end_stone", &[]);
    let stair_props = [
        ("facing", "east"),
        ("half", "top"),
        ("shape", "straight"),
        ("waterlogged", "false"),
    ];
    let stairs = w.id("stone_brick_stairs", &stair_props);
    let mossy = w.id("mossy_stone_brick_stairs", &stair_props);
    let slab = |name: &str| w.id(name, &[("type", "top"), ("waterlogged", "false")]);
    let (unoxidized, exposed, weathered, oxidized) = (
        slab("cut_copper_slab"),
        slab("exposed_cut_copper_slab"),
        slab("weathered_cut_copper_slab"),
        slab("oxidized_cut_copper_slab"),
    );
    let copper = w.id("copper_block", &[]);
    let exposed_copper = w.id("exposed_copper", &[]);
    let waxed_copper = w.id("waxed_copper_block", &[]);
    let air = b.air();
    let at = BlockPos::new(4, 8, 4);
    let below = at.offset(0, -1, 0);

    // Random ticks on an exposed slab until it weathers: what it became.
    let weather = |w: &mut World| {
        w.put(at, exposed);
        for _ in 0..2000 {
            b.sim(&mut w.level, &mut w.ticks)
                .random_tick_block(exposed, at);
            if w.get(at) != exposed {
                break;
            }
        }
        w.get(at)
    };
    // Lightning on an oxidized slab: what the struck block went back to.
    let strike = |w: &mut World| {
        w.put(at, oxidized);
        b.sim(&mut w.level, &mut w.ticks).lightning_clear_copper(at);
        w.get(at)
    };
    // Fire in the air above end stone: its spread odds, and whether the highest burn chance
    // burns the end stone.
    let fire = |w: &mut World| {
        w.put(at, air);
        w.put(below, end_stone);
        let spread = {
            let mut sim = b.sim(&mut w.level, &mut w.ticks);
            let spread = sim.fire_ignite_odds_at(at);
            sim.fire_check_burn_out(at, Direction::Down, 300, 0);
            spread
        };
        (spread, w.get(below) != end_stone)
    };
    let vanilla = |w: &mut World| {
        let sim = b.sim(&mut w.level, &mut w.ticks);
        assert_eq!(sim.ignite_odds(end_stone, below, Direction::Up), 0);
        assert_eq!(sim.burn_odds(end_stone, below, Direction::Up), 0);
        assert_eq!(sim.copper_next(exposed), Some(weathered));
        assert_eq!(sim.copper_next(copper), Some(exposed_copper));
        assert_eq!(sim.copper_stepped(oxidized, false), Some(weathered));
        assert_eq!(b.mapped_waxed(stairs), None);
        assert_eq!(b.mapped_waxed(copper), None);
        assert_eq!(b.mapped_unwaxed(mossy), None);
        assert_eq!(b.mapped_scraped(oxidized), None);
        assert!(b.is_waxed(waxed_copper) && !b.is_waxed(mossy));
        assert_eq!(fire(w), (0, false));
        assert_eq!(weather(w), weathered);
        assert_eq!(strike(w), unoxidized);
    };
    vanilla(&mut w);

    let flammable = json!({"ignite_odds": 5, "burn_odds": 300});
    let skips_a_stage = json!({"next_oxidation_stage": "minecraft:oxidized_cut_copper_slab"});
    let mossy_wax = json!({"waxed": "minecraft:mossy_stone_brick_stairs"});
    let rank = |name: &str| {
        b.table
            .block_by_name(name)
            .map_or(u32::MAX, |id| u32::from(id.0))
    };
    b.set_block_maps(Some(Arc::new(BlockMaps::from_values(
        [("minecraft:end_stone", &flammable)],
        [("minecraft:exposed_cut_copper_slab", &skips_a_stage)],
        [("minecraft:stone_brick_stairs", &mossy_wax)],
        rank,
    ))));
    {
        let sim = b.sim(&mut w.level, &mut w.ticks);
        assert_eq!(sim.ignite_odds(end_stone, below, Direction::Up), 5);
        assert_eq!(sim.burn_odds(end_stone, below, Direction::Up), 300);
        // The map's next stage, properties kept; an unmapped block keeps vanilla's.
        assert_eq!(sim.copper_next(exposed), Some(oxidized));
        assert_eq!(sim.copper_next(copper), Some(exposed_copper));
        // One scrape back goes through the derived inverse.
        assert_eq!(sim.copper_stepped(oxidized, false), Some(exposed));
    }
    assert_eq!(fire(&mut w), (5, true));
    // The fire hook comes before the map.
    w.level.fire_odds.insert((end_stone, None), (0, 0));
    assert_eq!(fire(&mut w), (0, false));
    w.level.fire_odds.clear();
    // A random tick weathers past weathered; lightning walks the derived inverse back to
    // exposed, then vanilla's naming the rest of the way.
    assert_eq!(weather(&mut w), oxidized);
    assert_eq!(strike(&mut w), unoxidized);
    // Honeycomb waxes to the map's twin; the axe takes the wax off and scrapes, properties kept.
    assert_eq!(b.mapped_waxed(stairs), Some(mossy));
    assert_eq!(b.mapped_unwaxed(mossy), Some(stairs));
    assert_eq!(b.mapped_scraped(oxidized), Some(exposed));
    assert!(b.is_waxed(mossy) && b.is_waxed(waxed_copper));
    assert_eq!(b.mapped_waxed(copper), None);

    // A reload whose packs give no maps installs none: vanilla's answers come back.
    b.set_block_maps(None);
    vanilla(&mut w);
}
