//! The `default_components_bound` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Items' default components are bound and readable (NeoForge's
    /// `DefaultDataComponentsBoundEvent`); `cause` says how, as `tags_updated`'s does. The
    /// place to build or rebuild caches derived from item defaults (stack sizes, food,
    /// tool rules): from now on the default-component reads return the published
    /// defaults, `modify_default_components`' edits included.
    ///
    /// `ServerDataLoad`: on the server once its data tables are built, on a world's first
    /// open and after `/reload`. `ClientPacketReceived`: on a client once it applied the
    /// tags its server sent, at login and for each update in play. Each fires right before
    /// `tags_updated` with the same cause; `cause.should_update_static_data()` is false
    /// for an integrated server's client, so data the two threads share is updated once.
    ///
    /// Runs in Reloading: published asset/data queries can rebind caches; world handles
    /// and transactions remain unavailable.
    #[hook(
        side = "both",
        contract = "listen",
        neoforge = "DefaultDataComponentsBoundEvent"
    )]
    #[notes = r##"## When the notice fires, per side and cause

`ServerDataLoad { reload: false }` fires once the server's data tables are
built, on a world's first open, before its chunks and players load;
`ServerDataLoad { reload: true }` fires after `/reload`, once the players were
sent the new data. `ClientPacketReceived { integrated_server }` fires on a
client once per set of tags its server sends, at login and for each update in
play: the client's applied registries are what changed there, and the notice
marks them in place. In single player both causes fire, each on its own
thread; `cause.should_update_static_data()` is false only for an integrated
server's client packet, so the data the two threads share is updated once, on
the server.

Each notice fires right before the same-cause `tags_updated`, after the
published defaults and the tags are readable and before any entity, block or
player is ticked with them. The read helpers (`item_default_component`,
`max_stack_size`, `max_damage`) answer the published defaults from then on,
`modify_default_components`' edits included. `modify_default_components`
itself is not replayed by reloads: a reader that must follow a reload rebuilds
here, which is why the notice also fires again after `/reload`.

Unlike NeoForge's client, Horizon's client does not rebind component defaults
on a tag packet: its defaults come from its own load. The client-side notice
therefore marks the applied tags' arrival rather than a rebinding; how often
it should fire, and its exact order against the client's component binding,
are the open questions this hook's review asks about."##]
    fn default_components_bound();
}
