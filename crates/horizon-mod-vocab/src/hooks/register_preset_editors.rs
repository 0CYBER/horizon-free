//! World-preset editor registration.

pub trait Hooks {
    /// Register client screens for world-preset keys once at startup. The first
    /// registration for a key wins. Invalid ids are refused. Unregistered presets
    /// retain their built-in editor, or their disabled Customize button.
    #[notes = r#"## World preset editors

`register_preset_editors` is the client registration-phase counterpart of NeoForge's
`RegisterPresetEditorsEvent`. Push `PresetEditor { preset, screen }` into `event.editors`.
The preset comes from the enabled data packs' `worldgen/world_preset` JSON; registering
an editor alone does not create a preset. Editor ids must be namespaced outside `minecraft`.
Add the preset to `data/minecraft/tags/worldgen/world_preset/normal.json` to offer it
in Create World's type selector; unlisted presets remain hidden.

```rust,ignore
fn register_preset_editors(
    &self,
    _: &RegistrationContext,
    event: &mut RegisterPresetEditorsEvent<'_>,
) {
    event.editors.push(PresetEditor {
        preset: "example:islands".into(),
        screen: "example:islands_editor".into(),
    });
}
```

Customize opens `world_preset_editor`, with Done and Cancel supplied by the game.
Use the existing screen render and input hooks for the custom presentation and controls.
`host.preset_editor()` identifies the selected editor; `host.preset_editor_dimensions()`
returns its working dimension stems. `host.set_preset_editor_dimensions(...)` replaces
that working copy after structural validation. Done commits it to world creation;
Cancel and Escape discard it. Neither path changes an existing world. Screen drawing
and input work before a world is loaded and use the same API on native and wasm.
Keep custom drawing above the bottom 33 GUI pixels reserved for Done and Cancel.
Dimension ids, stem structure and flat-layer height bounds are checked before accepting
an edit; registry resolution and generator-specific decoding still run when opening the world.
"#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "editors",
        context = "registration",
        neoforge = "RegisterPresetEditorsEvent"
    )]
    fn register_preset_editors();
}
