# Check biome tags

Use the world's live biome tags without loading chunks:

```rust,ignore
use mod_api::{Host, LevelRef};

fn is_forest(host: &dyn Host, level: LevelRef, biome: &str) -> Option<bool> {
    host.level_biome_in_tag(level, biome, "minecraft:is_forest")
}
```

The query includes data-pack biome tags and accepts a tag name with or without `#`.

| Result | Meaning |
|---|---|
| `Some(true)` | The known biome belongs to the tag. |
| `Some(false)` | The known biome is outside the tag, or the tag is undefined. |
| `None` | The biome, level or live biome-tag tables are unavailable. |
