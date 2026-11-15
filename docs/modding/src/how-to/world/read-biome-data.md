# Read biome data

Read the level's biome table without loading chunks. A stale level or unknown biome returns `None`.

```rust,ignore
use mod_api::{Host, LevelRef};

fn inspect(host: &dyn Host, level: LevelRef) {
    if let Some(effects) = host.level_biome_effects(level, "minecraft:plains") {
        let water_rgb = effects.water;
        // Sky and fog preserve omitted declarations as None. Water fog instead
        // uses 0x050533 when omitted. These are not dimension-resolved colours.
        let water_fog_rgb = effects.water_fog;
    }
    if let Some(categories) = host.level_biome_spawners(level, "minecraft:plains") {
        for category in categories {
            // Categories are sorted by name; entries retain biome-file order.
            let entries = category.entries;
        }
    }
}
```

The table includes data packs and biome modifiers. Spawners describe the biome's own natural spawn lists, not a position-specific potential-spawn query.
