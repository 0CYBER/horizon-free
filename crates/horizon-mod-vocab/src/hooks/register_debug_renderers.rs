//! The `register_debug_renderers` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The world-space debug renderers a mod has now (NeoForge's `RegisterDebugRenderersEvent`): push a
    /// `DebugRenderer` per renderer. Asked at each renderer list refresh, on the client.
    #[notes = r#"## Debug renderers

`Hooks::register_debug_renderers` is the client counterpart of NeoForge 26.3's
`RegisterDebugRenderersEvent`, and like it fires during vanilla's
`DebugRenderer.refreshRendererList`: once as the client starts, on each login, on the first view
of a world since the start, and on the next view of a world after the debug selection changes.
That is a status set in the debug options or by a key, a profile loaded, the F3 overlay opened or
closed, or reduced debug info changed. It is not asked on other views, nor on title or connect
screens. It receives a `RegistrationContext`, so it reads no world, and collects a `DebugRenderer`
per renderer.

Each refresh replaces the active list, so a mod pushes every renderer it has, each time, and may
rebuild its renderers' state here. Pushing an id does not list an option, since the option must
exist before any refresh. As in vanilla, the option is an entry registered once at start:
`Hooks::register_debug_entries` with `DebugGroup::Renderer` and the same id. It is listed in the debug options' renderer section, sorted by its id with
vanilla's, and is switched like them (`alwaysOn`, `inOverlay`, `never`), saved in
`debug-profile.json` under its full id. It starts off, and reduced debug info hides it, as it
hides vanilla's. A renderer whose entry is off is left out of the active list, and one with no
such entry is logged and left out.

While a renderer is active, `Hooks::debug_renderer_draw` is asked for each view of a world, after
`submit_custom_geometry`, with the renderer's id, the same `WorldRenderView` and the same
`WorldDrawContext`, so [its limits and rules](#custom-world-geometry) apply and are shared. Every
mod is asked about every id, so a mod ignores ids it does not own. Output clears every view and
title or connect screens ask no renderer, so nothing stays after leaving a world. A renderer that
is not active is never asked, so with none active a view only compares the selection's version."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "renderers",
        context = "registration",
        neoforge = "RegisterDebugRenderersEvent"
    )]
    fn register_debug_renderers();
}
