//! Client spectator post-effect registration.

pub trait Hooks {
    /// Register the screen effect spectating an entity type puts on, once at client startup
    /// (NeoForge's `RegisterEntitySpectatorShadersEvent`), as vanilla's creeper, spider and
    /// enderman do. The first registration for an entity type wins, vanilla's own counting as
    /// first, so a mod can't change what the vanilla mobs show; a skipped or malformed
    /// registration is logged. The effect applies while the first-person view follows that
    /// entity, and the third-person view or leaving it clears it.
    #[notes = r#"## Entity spectator shaders

`Hooks::register_entity_spectator_shaders` is the client registration-phase counterpart of NeoForge's
`RegisterEntitySpectatorShadersEvent`. Push `EntitySpectatorShader` records into `event.shaders`: the
spectated entity type and the post effect it puts on the screen. The post effect is an ordinary
post chain, `assets/<namespace>/post_effect/<path>.json`, which the mod's assets or a resource pack
supply, so its passes follow the same pack rules as vanilla's `minecraft:creeper` and
`minecraft:invert`; no shader code or GPU object crosses the mod API.

When the first-person view follows an entity (spectator mode, or a server's camera), the game
applies the registered effect for that entity's type (`GameRenderer.checkEntityPostEffect`). The
third-person view and leaving the camera clear it. The first registration for each entity type wins:
vanilla's creeper, spider, cave spider and enderman count as first, so a mod can't change what the
vanilla mobs show. A later registration for the same type, and one whose ids aren't namespaced, is
logged and skipped. Registered chains compile in the background with vanilla's, from the first frame
in a world.

```rust,ignore
fn register_entity_spectator_shaders(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterEntitySpectatorShadersEvent<'_>,
) {
    event.shaders.push(EntitySpectatorShader {
        entity: "example:wisp".into(),
        shader: "example:wisp_vision".into(),
    });
}
```"#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "shaders",
        context = "registration",
        neoforge = "RegisterEntitySpectatorShadersEvent"
    )]
    fn register_entity_spectator_shaders();
}
