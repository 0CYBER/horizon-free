# The typed API from injection listeners

Use the typed API from an `[[inject]]` listener. An API point on a `Game` method supplies the side's
`Host`, through which you access attachments and handles. The listener contracts are in
[Injections](../injections.md#declaration). Source-level edits use the separate
[Mixins](../native-route/2-mixins.md) mechanism, in horizon-mixins' vocabulary.

A catalog hook's trait method, event and return make up its `hook_version` contract
([Named hook points](../../how-to/entities/named-hook-points.md)). These per-hook contract versions are catalog metadata, not minimum game
versions. Catalog hooks, `Host` calls, `[[inject]]` declarations and `[[native.*]]` tables all build against the
game's exact source revision pinned in `[native]`. Rebuild the mod when that contract changes.

Every `impl Hooks` must have `#[mod_api::horizon_mod]`. The attribute derives
subscriptions from the methods implemented in that block, so dispatch knows them at installation.

Container open notifications run synchronously after a successful open, not at the next tick. Failed opens and the player's own inventory do not notify. Replacing an open menu closes the previous menu before announcing its replacement. This corrects the documented boundary and keeps the hook version unchanged.
