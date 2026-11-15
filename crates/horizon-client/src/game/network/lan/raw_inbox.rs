//! Reborrow live world state only after a raw root has completed.
use super::*;

impl Game<'_> {
    pub(super) fn resume_raw_inbox(
        &mut self,
        hub: &mut Hub,
        entities: &[horizon_client::lan::entity::EntityState],
        spawn: [f64; 3],
    ) -> Vec<HostEvent> {
        // Raw transactions finished above. Reborrow current world state only
        // now; no callback captures a TickCtx or an as_guest world swap.
        let clock_rate = self.network_clock_rate();
        let mut ctx = TickCtx {
            sea_level: self.end.locator().map_or(63, |terrain| terrain.sea_level()),
            world: &mut self.world,
            entities,
            next_entity: &mut self.next_entity,
            spawn,
            spawn_angle: self.level.world_spawn_angle,
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            tick_steps: self.clock.step_state(),
            collision: Some(self.collision),
            border: &self.border,
            streaming: Some(&self.streaming),
        };
        hub.drain_packets(&mut ctx)
    }
}

impl Game<'_> {
    pub(super) fn drain_raw_turns(
        &mut self,
        mut host: LanHost,
        mut events: Vec<HostEvent>,
        entities: &[horizon_client::lan::entity::EntityState],
        spawn: [f64; 3],
    ) -> Option<LanHost> {
        loop {
            let back = horizon_client::spans::time("lan_events", || {
                self.lan_process_events(host, events, entities, spawn)
            })?;
            host = back;
            if host.hub.payload_deferred() || !host.hub.packets_paused() {
                return Some(host);
            }
            events = self.resume_raw_inbox(&mut host.hub, entities, spawn);
        }
    }
}

impl Game<'_> {
    pub(super) fn receive_raw_delivery(
        &mut self,
        mut delivery: Box<horizon_client::raw_payloads::Delivery>,
    ) {
        let finished =
            self.receive_server_payload(delivery.endpoint.clone(), &mut delivery.incoming);
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        if !finished {
            host.hub.defer_raw_payload(delivery);
        } else {
            host.hub.finish_raw_payload();
        }
    }
}
