//! Ordered raw/ordinary receiving turns and physical input ownership.
use super::*;

pub(super) struct PacketDrain {
    pub(super) conns: Vec<u32>,
    pub(super) next: usize,
    pub(super) started: bool,
    // Ordinary action deferral survives raw yields until this guest turn ends.
    pub(super) movement_held: bool,
}

pub(super) struct DeferredPayload {
    delivery: Option<Box<crate::raw_payloads::Delivery>>,
    retrying: bool,
    movement_held: bool,
}

impl Hub {
    /// Handle the players in this level's packets (`ServerGamePacketListenerImpl`
    /// for each), before the level's scheduled block ticks so a place landed in
    /// the same tick as a falling block's schedule still sees the support
    /// (arismoko/horizon#4510). `tick_level` then syncs what those packets (and the block ticks)
    /// changed.
    pub fn drain_packets(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        let mut events = Vec::new();
        let mut pass = self.packet_drain.take().unwrap_or_else(|| PacketDrain {
            conns: self
                .guests
                .iter()
                .filter(|g| self.here(g) || (g.held && self.watches(g)))
                .map(|g| g.conn)
                .collect(),
            next: 0,
            started: false,
            movement_held: false,
        });
        while let Some(&conn) = pass.conns.get(pass.next) {
            if let Some(held) = self.deferred_payloads.get(&Some(conn)) {
                if held.retrying || held.delivery.is_some() {
                    pass.next += 1;
                    pass.started = false;
                    pass.movement_held = false;
                    continue;
                }
                pass.started = true;
                pass.movement_held = held.movement_held;
                self.deferred_payloads.remove(&Some(conn));
            }
            let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
                pass.next += 1;
                pass.started = false;
                pass.movement_held = false;
                continue;
            };
            // A raw root may close the physical endpoint before its Left event
            // reaches the next frozen poll. Do not acknowledge or apply later
            // input in this incarnation while waiting for that notification.
            if self.discard_closed_input(i) {
                self.guests.update(i, |g| g.move_packet_queue.clear());
                pass.next += 1;
                pass.started = false;
                pass.movement_held = false;
                continue;
            }
            if !self.here(&self.guests[i])
                && !(self.guests[i].held && self.watches(&self.guests[i]))
            {
                pass.next += 1;
                pass.started = false;
                pass.movement_held = false;
                continue;
            }
            let frozen = self.guests[i].handover.is_some() || self.guests[i].held;
            if frozen {
                self.keep_acknowledgements(i, &mut events);
            }
            if !self.guests[i].move_packet_queue.is_empty() {
                if frozen || !self.move_packets_listened {
                    self.guests.update(i, |g| g.move_packet_queue.clear());
                } else if !pass.started {
                    // The packets over last tick's budget go ahead of this tick's, in the guest's
                    // own level pass.
                    self.tell_move_packets(i, &mut events);
                }
            }
            if self.process_inbox(i, ctx, &mut events, !pass.started, &mut pass.movement_held) {
                pass.started = true;
                self.packet_drain = Some(pass);
                return events;
            }
            if !frozen && self.here(&self.guests[i]) {
                self.tick_floating(i, &mut events);
            }
            pass.next += 1;
            pass.started = false;
            pass.movement_held = false;
        }
        events
    }

    /// A raw boundary must finish in Game before continuing this same level pass.
    /// Resuming drains only the frozen inbox turn: no poll, maintenance or world tick.
    pub fn packets_paused(&self) -> bool {
        self.packet_drain.is_some()
    }

    /// A refused guest call retains the same receiving root at the queue head.
    pub fn defer_raw_payload(&mut self, delivery: Box<crate::raw_payloads::Delivery>) {
        let movement_held = self
            .packet_drain
            .as_ref()
            .is_some_and(|pass| pass.movement_held);
        let held = self
            .deferred_payloads
            .entry(delivery.conn)
            .or_insert(DeferredPayload {
                delivery: None,
                retrying: false,
                movement_held,
            });
        held.retrying = false;
        held.delivery = Some(delivery);
    }

    pub fn payload_deferred(&self) -> bool {
        self.deferred_payloads
            .values()
            .any(|held| held.retrying || held.delivery.is_some())
    }

    pub fn finish_raw_payload(&mut self, conn: Option<u32>) {
        if let Some(held) = self.deferred_payloads.get_mut(&conn) {
            held.retrying = false;
        }
    }

    pub(super) fn begin_payload_poll(&mut self) -> Vec<HostEvent> {
        let mut events = std::mem::take(&mut self.pending_kicks);
        // Departed physical connections no longer own a receiving turn or its credit.
        self.deferred_payloads.retain(|conn, _| {
            conn.is_none_or(|conn| self.guests.iter().any(|guest| guest.conn == conn))
        });
        for held in self.deferred_payloads.values_mut() {
            if let Some(delivery) = held.delivery.take() {
                held.retrying = true;
                events.push(HostEvent::RawPayload(delivery));
            }
        }
        events
    }

    pub(super) fn bind_raw_player(&self, guest: &Guest, entity_id: i32) {
        if let Some(epoch) = self.payload_player_epoch {
            guest
                .sender
                .payload_transport()
                .bind_player(mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(epoch, entity_id as u32),
                    guest.uuid,
                ));
        }
    }

    pub(super) fn poll_raw_payload(
        &mut self,
        delivery: Box<crate::raw_payloads::Delivery>,
        credit: crate::net::ingress::ReceiveLease<NetEvent>,
        events: &mut Vec<HostEvent>,
        credits: &mut Vec<crate::net::ingress::ReceiveLease<NetEvent>>,
    ) {
        if let Some(conn) = delivery.conn {
            self.update_guest(conn, |g| {
                if g.left_reason.is_empty() {
                    g.inbox.push_back(InboxItem::Raw(delivery, credit));
                }
            });
        } else {
            events.push(HostEvent::RawPayload(delivery));
            credits.push(credit);
        }
    }
}

impl Hub {
    pub(super) fn discard_closed_input(&mut self, i: usize) -> bool {
        // The I/O owner can also close concurrently between packets.
        if !self.guests[i].left_reason.is_empty()
            || self.guests[i].sender.payload_transport().closed()
        {
            let discarded = self.guests.update(i, |g| std::mem::take(&mut g.inbox));
            drop(discarded);
            true
        } else {
            false
        }
    }
}

pub(super) fn yield_raw(
    delivery: Box<crate::raw_payloads::Delivery>,
    credit: crate::net::ingress::ReceiveLease<NetEvent>,
    events: &mut Vec<HostEvent>,
    credits: &mut Vec<crate::net::ingress::ReceiveLease<NetEvent>>,
) -> bool {
    credits.push(credit);
    if !delivery.incoming.epoch.active() {
        return false;
    }
    events.push(HostEvent::RawPayload(delivery));
    events.push(HostEvent::InputCredits(std::mem::take(credits)));
    true
}
