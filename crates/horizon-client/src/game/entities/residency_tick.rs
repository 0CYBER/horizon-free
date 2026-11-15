//! Complete residency journals on both logical-server and remote-client ticks.
use super::Game;

impl Game<'_> {
    pub(super) fn with_residency_tick(
        &mut self,
        tick: impl FnOnce(&mut Self) -> super::Result<()>,
    ) -> super::Result<()> {
        use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
        let result = catch_unwind(AssertUnwindSafe(|| tick(self)));
        // Always drain and expire pass markers, preserving the original panic
        // when interrupted actor work and completion both fail.
        let finished = catch_unwind(AssertUnwindSafe(|| {
            self.resident_entities.table.get_mut().end_tick()
        }));
        match result {
            Err(payload) => resume_unwind(payload),
            Ok(result) => {
                let finished = match finished {
                    Ok(finished) => finished,
                    Err(payload) => resume_unwind(payload),
                };
                result?;
                finished
                    .map_err(|error| format!("unfinished entity residency pass: {error:?}").into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_client_game;
    use horizon_client::mobs::Mob;
    use horizon_core::geometry::Vec3;

    #[test]
    #[ignore = "daily"]
    fn client_tick_drains_actual_bound_owner_events_without_a_lookup() {
        with_client_game(|game| {
            let (client, server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(client, "residency-client").unwrap();
            game.remote.as_mut().unwrap().logged_in = true;
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let journal = game.resident_entities.table.borrow().producer();
            for id in 800..820 {
                game.mobs.push(Mob::new(id, kind.clone(), Vec3::ZERO, 0.0));
            }
            game.tick().unwrap();
            let mut remaining = Vec::new();
            journal.drain_into(&mut remaining);
            assert!(
                remaining.is_empty(),
                "remote tick left owner events buffered"
            );
            game.mobs.clear();
            let result = game.with_residency_tick(|_| Err("interrupted client tick".into()));
            assert!(result.is_err());
            journal.drain_into(&mut remaining);
            assert!(
                remaining.is_empty(),
                "error return left owner events buffered"
            );
            server.shutdown();
        });
    }
}
