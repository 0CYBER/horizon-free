//! The `register_sprite_sources` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The atlas sprite source types a mod adds (NeoForge's `RegisterSpriteSourcesEvent`): push
    /// each type id, `<mod id>:<path>`. Asked once at start on the client, before resources load.
    /// An entry of an atlas definition's `sources` whose `type` is a registered id is answered by
    /// `sprite_source` at its place in the list, on every load and reload. An id in vanilla's
    /// namespace, not of that form, or registered already is logged and left out.
    #[notes = r#"## Sprite sources

An atlas definition (`assets/<namespace>/atlases/<path>.json`) lists the sources that give the atlas its
sprites: vanilla's `directory`, `single`, `filter`, `unstitch` and `paletted_permutations`. A mod adds a
source type of its own (NeoForge's `RegisterSpriteSourcesEvent`) in two steps, as an item tint source:
`Hooks::register_sprite_sources` pushes the type's id once at start, before resources load, and
`Hooks::sprite_source` answers each entry of that type when a definition is read, on every load and
reload.

```rust,ignore
fn register_sprite_sources(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterSpriteSourcesEvent<'_>,
) {
    event.sources.push("example:checker".into());
}

fn sprite_source(&self, _context: &RegistrationContext, event: &mut SpriteSourceEvent<'_>) {
    if event.source != "example:checker" {
        return;
    }
    // A JSON number arrives as the narrowest tag that holds it: `8` is a `Tag::Byte`.
    let side = match event.config.get("size") {
        Some(Tag::Byte(size)) => (*size).clamp(1, 64) as u32,
        _ => 16,
    };
    let rgba = (0..side * side)
        .flat_map(|i| if (i / side + i % side) % 2 == 0 { [255, 0, 255, 255] } else { [0, 0, 0, 255] })
        .collect();
    event.output.push(SpriteSourceOutput::Pixels {
        sprite: Identifier::parse("example:checker").unwrap(),
        width: side,
        height: side,
        rgba,
    });
}
```

Use it as an entry of a definition's `sources`, `{"type": "example:checker", "size": 8}`: the whole object
reaches the hook as `event.config`, `event.atlas` names the atlas, and `event.listed` holds the sprites the
entries before it listed. A source pushes `SpriteSourceOutput`s, which apply in order as vanilla's
`SpriteSource.Output` calls do: `Texture` adds a texture of the resources as a sprite, `Pixels` adds pixels
the mod made, and `Remove` drops a listed sprite (vanilla's `removeAll`, by id). A sprite of an id listed
already replaces the earlier one, as a built-in source's does.

Sprite sources apply to the atlases built from their definitions: the atlases mods register
(`register_texture_atlases`). An entry whose type no mod registered fails its definition, as an unknown type
fails it in vanilla, and the log names the type and the entry's place. A `Texture` the resources lack, and
`Pixels` whose length isn't `width * height * 4` or whose side is over 32768 pixels, are logged and skipped
one by one."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "sources",
        context = "registration",
        neoforge = "RegisterSpriteSourcesEvent"
    )]
    fn register_sprite_sources();
}
