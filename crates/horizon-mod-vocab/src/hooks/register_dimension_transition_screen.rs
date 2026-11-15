//! Client dimension-loading screen registration.

pub trait Hooks {
    /// Register loading screens once at client startup (NeoForge's
    /// `RegisterDimensionTransitionScreenEvent`). Conditional source/destination pairs take
    /// priority over incoming registrations, which take priority over outgoing registrations.
    /// Duplicate transition registrations keep the first; invalid ids are logged and skipped.
    /// An unmatched transition uses vanilla's screen and every screen uses the existing tracker.
    #[notes = r#"## Dimension transition screens

`Hooks::register_dimension_transition_screen` is the client registration-phase counterpart of
NeoForge's `RegisterDimensionTransitionScreenEvent`. Push `DimensionTransitionScreen` records into
`event.screens`, with a namespaced screen id and `DimensionTransition::Incoming(dimension)`,
`Outgoing(dimension)` or `Conditional { to, from }`. Conditional pairs win over incoming, then
outgoing. The first registration for each transition wins; malformed ids and screen ids in the
`minecraft` namespace are logged and skipped. Dimension ids are resolved once during registration.

The screen is selected once at `ClientPacketListener.startWaitingForNewLevel`, including login
(where there is no source dimension) and respawn. Instead of a Java screen factory, the selected
id reaches `Hooks::dimension_transition_screen_draw` with its reason and current optional server
progress. Draw the complete presentation using `event.draw`; ignore screen ids you do not own.
The existing level-load tracker owns readiness, the player-section wait, timeout, and closing.
No registration changes any of those rules. An unmatched transition draws vanilla's screen.

```rust,ignore
fn register_dimension_transition_screen(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterDimensionTransitionScreenEvent<'_>,
) {
    event.screens.push(DimensionTransitionScreen {
        screen: "example:arrival".into(),
        transition: DimensionTransition::Incoming("example:moon".into()),
    });
}

fn dimension_transition_screen_draw(
    &self,
    _host: &dyn ClientHost,
    event: &mut DimensionTransitionScreenDrawEvent<'_>,
) {
    if event.screen == "example:arrival" {
        let [w, h] = event.draw.size();
        event.draw.fill([0.0, 0.0, w, h], 0xFF101030);
    }
}
```"#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "screens",
        context = "registration",
        neoforge = "RegisterDimensionTransitionScreenEvent"
    )]
    fn register_dimension_transition_screen();
}
