//! `/swing`.
use super::entity::{EntityHost, EntityRef, holder, num};
use super::host::R;
use horizon_client::{command::Context, scoreboard_cmd::Tracker, text::Text};

/// What `/swing` needs from the game.
pub trait SwingHost: EntityHost {
    /// The entity the command source implies when no target is named.
    fn implied_entity(&self) -> R<EntityRef>;
    /// Makes a living entity swing its arm; false for an entity that cannot swing.
    fn swing_entity(&mut self, entity: EntityRef) -> bool;
}

/// `/swing [<targets>]`.
pub fn swing(host: &mut dyn SwingHost, ctx: &Context) -> R<i32> {
    let targets = if ctx.get("targets").is_some() {
        host.select_entities(ctx, "targets", false)?
    } else {
        vec![host.implied_entity()?]
    };
    let mut tracker = Tracker::default();
    for t in targets {
        if !host.swing_entity(t) {
            continue;
        }
        let h = holder(host.entity_name(t));
        tracker.mark(&h);
    }
    let (message, total) = tracker.message(
        false,
        Some(host.error("commands.swing.failed.notliving", &[])),
        |h, _| {
            Text::translate(
                "commands.swing.success.single",
                std::slice::from_ref(&h.text),
            )
        },
        |c, _| Text::translate("commands.swing.success.multiple", &[num(c)]),
    )?;
    host.feedback(message);
    Ok(total)
}
