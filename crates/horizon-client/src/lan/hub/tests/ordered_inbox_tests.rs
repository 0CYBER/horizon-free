//! Ordered Hub/action-stream tests. No prepared resources or game process.
use super::*;

fn with_input_context<T>(run: impl FnOnce(&mut TickCtx<'_>) -> T) -> T {
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    run(&mut TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    })
}

fn publish_packet(hub: &Hub, packet: S) {
    let guest = &hub.guests[0];
    assert!(guest.sender.undrained.handed(0));
    hub.lan
        .shared
        .events
        .send(NetEvent::Packet(guest.conn, Box::new(packet), 0))
        .unwrap();
}

#[test]
fn provider_credit_survives_poll_inbox_and_derived_action_segment() {
    let (mut hub, _out) = hub_with_a_guest();
    let provider = hub.lan.shared.events.clone();
    publish_packet(&hub, S::Punch(horizon_protocol::packets::Empty));
    let reserved = provider.stats().reserved_bytes;
    assert!(reserved > 0);
    with_input_context(|ctx| {
        drop(hub.poll(ctx));
        assert_eq!(provider.stats().queued_items, 0);
        assert_eq!(provider.stats().reserved_bytes, reserved);
        let events = hub.drain_packets(ctx);
        assert!(events.iter().any(
            |e| matches!(e, HostEvent::Packet(_, packet) if matches!(&**packet, S::Punch(_)))
        ));
        assert!(hub.guests[0].inbox.is_empty());
        assert_eq!(provider.stats().reserved_bytes, reserved);
        drop(events); // Game finished or discarded the complete segment.
        assert_eq!(provider.stats().reserved_bytes, 0);
    });
}

#[test]
fn leaving_physical_guest_keeps_then_discards_its_queued_input_credit() {
    let (mut hub, _out) = hub_with_a_guest();
    let provider = hub.lan.shared.events.clone();
    publish_packet(&hub, S::Punch(horizon_protocol::packets::Empty));
    let conn = hub.guests[0].conn;
    provider
        .send(NetEvent::Left(conn, "test departure".into()))
        .unwrap();
    with_input_context(|ctx| {
        let events = hub.poll(ctx);
        assert!(hub.guests.is_empty());
        assert!(provider.stats().reserved_bytes > 0);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, HostEvent::Left(g) if g.conn == conn && g.inbox.len() == 1))
        );
        drop(events);
        assert_eq!(provider.stats().reserved_bytes, 0);
    });
}

#[test]
fn derived_action_replies_wait_for_the_next_frozen_receiving_turn() {
    let (mut hub, _out) = hub_with_a_guest();
    let provider = hub.lan.shared.events.clone();
    publish_packet(&hub, S::Punch(horizon_protocol::packets::Empty));
    with_input_context(|ctx| {
        drop(hub.poll(ctx));
        let events = hub.drain_packets(ctx);
        for event in events {
            if matches!(event, HostEvent::Packet(_, _)) {
                publish_packet(&hub, S::Punch(horizon_protocol::packets::Empty));
            }
        }
        assert_eq!(provider.stats().queued_items, 1);
        assert!(hub.guests[0].inbox.is_empty());
        drop(hub.drain_packets(ctx)); // No new receiving turn, no feedback drain.
        assert_eq!(provider.stats().queued_items, 1);
        drop(hub.poll(ctx));
        drop(hub.drain_packets(ctx));
        assert_eq!(provider.stats().reserved_bytes, 0);
    });
}

fn raw_hub(
    transport: Arc<crate::raw_payloads::Transport>,
) -> (Hub, std::sync::mpsc::Receiver<crate::lan::Out>) {
    let (mut hub, _old_output) = hub_with_a_guest();
    let (tx, out) = std::sync::mpsc::channel();
    let sender =
        crate::lan::PlayerSender::new_with_payload_transport(tx, Arc::default(), transport);
    hub.guests.update(0, |g| {
        g.sender = sender;
        g.loaded = true;
        g.load_timeout = 0;
    });
    (hub, out)
}

fn publish_raw(hub: &Hub, bytes: Vec<u8>) {
    let guest = &hub.guests[0];
    let delivery =
        crate::raw_payloads::test_support::delivery(guest.conn, guest.sender.clone(), bytes);
    hub.lan
        .shared
        .events
        .send(NetEvent::RawPayload(delivery))
        .unwrap();
}

fn rotation(yaw: f32) -> S {
    S::MovePlayerRot(play::MovePlayerRot {
        yaw,
        pitch: 0.0,
        flags: 1,
    })
}

#[test]
fn raw_boundary_orders_earlier_actions_callback_effect_and_later_movement() {
    crate::raw_payloads::test_support::with_peer(|transport| {
        let (mut hub, _out) = raw_hub(transport.clone());
        let provider = hub.lan.shared.events.clone();
        publish_packet(&hub, rotation(10.0));
        publish_packet(&hub, S::Punch(horizon_protocol::packets::Empty));
        publish_raw(&hub, vec![7]);
        publish_packet(&hub, rotation(30.0));
        with_input_context(|ctx| {
            drop(hub.poll(ctx));
            let tick = hub.ticks;
            let events = hub.drain_packets(ctx);
            assert!(hub.packets_paused());
            assert_eq!(
                hub.guests[0].yaw, 10.0,
                "later rotation must not apply before the callback"
            );
            let mut trace = Vec::new();
            for event in events {
                match event {
                    HostEvent::Packet(_, packet) if matches!(&*packet, S::Punch(_)) => {
                        trace.push("earlier action")
                    }
                    HostEvent::RawPayload(delivery) => {
                        assert_eq!(delivery.conn, Some(hub.guests[0].conn));
                        assert!(Arc::ptr_eq(delivery.endpoint.transport(), &transport));
                        assert!(delivery.incoming.epoch.active());
                        assert_eq!(
                            delivery.incoming.id.0,
                            crate::raw_payloads::test_support::CHANNEL
                        );
                        trace.push("raw boundary");
                        // This is the Game-owned action-stream boundary, not an
                        // invented IPayloadContext. The prepared-data Game test
                        // exercises the registered handler/root transaction itself.
                        hub.guests.update(0, |g| {
                            g.yaw = 20.0;
                        });
                        assert_eq!(hub.guests[0].yaw, 20.0);
                        trace.push("callback effect");
                        // A reply arriving during the callback cannot be read by resume.
                        publish_packet(&hub, rotation(99.0));
                    }
                    _ => {}
                }
            }
            assert_eq!(trace, ["earlier action", "raw boundary", "callback effect"]);
            drop(hub.drain_packets(ctx));
            assert!(!hub.packets_paused());
            assert_eq!(hub.guests[0].yaw, 30.0);
            assert_eq!(hub.ticks, tick, "resume must not poll or run maintenance");
            assert_eq!(provider.stats().queued_items, 1);
            drop(hub.poll(ctx));
            drop(hub.drain_packets(ctx));
            assert_eq!(hub.guests[0].yaw, 99.0);
            assert_eq!(provider.stats().reserved_bytes, 0);
        });
    });
}

#[test]
fn raw_resume_initializes_guest_and_ticks_floating_only_once() {
    crate::raw_payloads::test_support::with_peer(|transport| {
        let (mut hub, _out) = raw_hub(transport);
        hub.guests.update(0, |g| {
            g.load_timeout = 7;
            g.client_is_floating = true;
            g.above_ground_ticks = 0;
        });
        publish_raw(&hub, vec![1]);
        publish_raw(&hub, vec![2]);
        with_input_context(|ctx| {
            drop(hub.poll(ctx));
            for expected_paused in [true, true, false] {
                drop(hub.drain_packets(ctx));
                assert_eq!(hub.packets_paused(), expected_paused);
                assert_eq!(hub.guests[0].load_timeout, 6);
                assert_eq!(
                    hub.guests[0].above_ground_ticks,
                    i32::from(!expected_paused)
                );
            }
        });
    });
}

#[test]
fn frozen_movement_does_not_discard_registered_raw_or_consume_later_movement() {
    crate::raw_payloads::test_support::with_peer(|transport| {
        let (mut hub, _out) = raw_hub(transport);
        hub.guests.update(0, |g| {
            g.held = true;
            g.placed = false;
        });
        publish_packet(&hub, rotation(10.0));
        publish_raw(&hub, vec![1]);
        publish_packet(&hub, rotation(30.0));
        with_input_context(|ctx| {
            let before = hub.guests[0].yaw;
            drop(hub.poll(ctx));
            let events = hub.drain_packets(ctx);
            assert!(events.iter().any(|e| matches!(e, HostEvent::RawPayload(_))));
            assert_eq!(hub.guests[0].yaw, before);
            assert_eq!(
                hub.guests[0].inbox.len(),
                1,
                "filter must not cross raw boundary"
            );
            drop(events);
            drop(hub.drain_packets(ctx));
            assert!(hub.guests[0].inbox.is_empty());
            assert_eq!(hub.guests[0].yaw, before);
        });
    });
}

#[test]
fn cancelled_epoch_and_left_physical_connection_never_yield_raw_callbacks() {
    crate::raw_payloads::test_support::with_peer(|transport| {
        let (mut hub, _out) = raw_hub(transport.clone());
        let provider = hub.lan.shared.events.clone();
        publish_raw(&hub, vec![1]);
        transport.close();
        with_input_context(|ctx| {
            drop(hub.poll(ctx));
            let events = hub.drain_packets(ctx);
            assert!(!events.iter().any(|e| matches!(e, HostEvent::RawPayload(_))));
            drop(events);
            assert_eq!(provider.stats().reserved_bytes, 0);
        });
    });
    crate::raw_payloads::test_support::with_peer(|transport| {
        let (mut hub, _out) = raw_hub(transport);
        let provider = hub.lan.shared.events.clone();
        let conn = hub.guests[0].conn;
        publish_raw(&hub, vec![1]);
        provider
            .send(NetEvent::Left(conn, "left physical incarnation".into()))
            .unwrap();
        with_input_context(|ctx| {
            let events = hub.poll(ctx);
            assert!(!events.iter().any(|e| matches!(e, HostEvent::RawPayload(_))));
            drop(events);
            assert_eq!(provider.stats().reserved_bytes, 0);
            drop(hub.drain_packets(ctx));
            assert!(!hub.packets_paused());
        });
    });
}

#[test]
fn large_private_raw_and_thousands_of_tiny_records_use_byte_credit_not_wire_caps() {
    crate::raw_payloads::test_support::with_peer(|transport| {
        let (mut hub, _out) = raw_hub(transport.clone());
        let provider = hub.lan.shared.events.clone();
        publish_raw(&hub, vec![0; 9 << 20]);
        for _ in 0..2000 {
            publish_raw(&hub, vec![1]);
        }
        let reserved = provider.stats().reserved_bytes;
        assert!(transport.reserve_ingress(8 << 20).is_err());
        with_input_context(|ctx| {
            drop(hub.poll(ctx));
            assert_eq!(provider.stats().reserved_bytes, reserved);
            assert_eq!(hub.guests[0].inbox.len(), 2001);
            let first = hub.drain_packets(ctx);
            assert_eq!(
                provider.stats().reserved_bytes,
                reserved,
                "dequeue must not refund either credit"
            );
            assert!(transport.reserve_ingress(8 << 20).is_err());
            assert_eq!(
                first
                    .iter()
                    .filter(|e| matches!(e, HostEvent::RawPayload(_)))
                    .count(),
                1
            );
            drop(first);
            let mut callbacks = 1;
            while hub.packets_paused() {
                let events = hub.drain_packets(ctx);
                callbacks += events
                    .iter()
                    .filter(|e| matches!(e, HostEvent::RawPayload(_)))
                    .count();
                drop(events);
            }
            assert_eq!(callbacks, 2001);
            assert_eq!(provider.stats().reserved_bytes, 0);
            drop(
                transport
                    .reserve_ingress(9 << 20)
                    .expect("physical credit refunded after consumption"),
            );
        });
    });
}

#[test]
fn raw_disconnect_rejects_later_input_before_left_notification_is_polled() {
    crate::raw_payloads::test_support::with_peer(|transport| {
        let (mut hub, _out) = raw_hub(transport.clone());
        let provider = hub.lan.shared.events.clone();
        publish_packet(&hub, rotation(10.0));
        publish_raw(&hub, vec![1]);
        publish_packet(&hub, rotation(30.0));
        publish_packet(&hub, S::Punch(horizon_protocol::packets::Empty));
        with_input_context(|ctx| {
            drop(hub.poll(ctx));
            let first = hub.drain_packets(ctx);
            assert!(hub.packets_paused());
            assert_eq!(hub.guests[0].yaw, 10.0);
            let mut disconnected = false;
            for event in first {
                if let HostEvent::RawPayload(delivery) = event {
                    // The real endpoint path closes Transport immediately. This
                    // fixture does not invent an early NetEvent::Left for Hub.
                    delivery
                        .endpoint
                        .disconnect(mod_api::Text::literal("test raw disconnect"));
                    disconnected = true;
                }
            }
            assert!(disconnected && transport.closed());
            assert!(
                hub.guests[0].left_reason.is_empty(),
                "Left has not been polled"
            );
            assert_eq!(hub.guests[0].inbox.len(), 2);
            assert!(provider.stats().reserved_bytes > 0);
            let resumed = hub.drain_packets(ctx);
            assert!(
                !resumed.iter().any(|event| matches!(
                    event,
                    HostEvent::Packet(_, _) | HostEvent::RawPayload(_)
                ))
            );
            drop(resumed);
            assert!(!hub.packets_paused());
            assert_eq!(hub.guests[0].yaw, 10.0, "later movement must not apply");
            assert!(hub.guests[0].inbox.is_empty());
            assert_eq!(
                provider.stats().reserved_bytes,
                0,
                "discarded input must refund all generic credits"
            );
        });
    });
}

#[test]
fn raw_never_overtakes_deferred_movement_and_cannot_reset_its_next_tick_deferral() {
    for raw_before_move in [false, true] {
        crate::raw_payloads::test_support::with_peer(|transport| {
            let (mut hub, _out) = raw_hub(transport);
            publish_packet(&hub, S::Attack(play::Attack { id: 7 }));
            publish_packet(&hub, S::Punch(horizon_protocol::packets::Empty));
            if raw_before_move {
                publish_raw(&hub, vec![1]);
            }
            publish_packet(&hub, rotation(10.0));
            if !raw_before_move {
                publish_raw(&hub, vec![1]);
            }
            with_input_context(|ctx| {
                drop(hub.poll(ctx));
                let first = hub.drain_packets(ctx);
                assert_eq!(
                    hub.guests[0].yaw, 0.0,
                    "action must defer subsequent movement"
                );
                assert!(first.iter().any(|e| matches!(e, HostEvent::Packet(_, packet) if matches!(&**packet, S::Attack(_)))));
                let raw_count = first
                    .iter()
                    .filter(|e| matches!(e, HostEvent::RawPayload(_)))
                    .count();
                assert_eq!(
                    raw_count,
                    usize::from(raw_before_move),
                    "raw cannot overtake a prior deferred move"
                );
                drop(first);
                if raw_before_move {
                    assert!(hub.packets_paused());
                    drop(hub.drain_packets(ctx));
                    assert_eq!(
                        hub.guests[0].yaw, 0.0,
                        "raw resume must retain the action's next-tick movement hold"
                    );
                }
                assert!(!hub.packets_paused());
                assert!(!hub.guests[0].inbox.is_empty());
                // Only the next receiving/tick turn may apply the deferred move.
                drop(hub.poll(ctx));
                let second = hub.drain_packets(ctx);
                assert_eq!(hub.guests[0].yaw, 10.0);
                assert_eq!(
                    second
                        .iter()
                        .filter(|e| matches!(e, HostEvent::RawPayload(_)))
                        .count(),
                    usize::from(!raw_before_move)
                );
                drop(second);
                if hub.packets_paused() {
                    drop(hub.drain_packets(ctx));
                }
                assert!(!hub.packets_paused());
                assert!(hub.guests[0].inbox.is_empty());
                assert_eq!(hub.lan.shared.events.stats().reserved_bytes, 0);
            });
        });
    }
}

#[test]
fn busy_raw_root_keeps_the_head_and_yields_until_the_next_receiving_turn() {
    crate::raw_payloads::test_support::with_peer(|transport| {
        let (mut hub, _out) = raw_hub(transport);
        publish_raw(&hub, vec![1]);
        publish_packet(&hub, rotation(30.0));
        publish_raw(&hub, vec![2]);
        with_input_context(|ctx| {
            drop(hub.poll(ctx));
            let mut first = None;
            for event in hub.drain_packets(ctx) {
                if let HostEvent::RawPayload(delivery) = event {
                    first = Some(delivery);
                }
            }
            let first = first.unwrap();
            let credit = Arc::downgrade(&first.incoming.lease);
            hub.defer_raw_payload(first);
            assert!(hub.payload_deferred());
            assert!(
                hub.drain_packets(ctx).is_empty(),
                "Busy must yield, not spin the frozen pass"
            );
            assert_ne!(hub.guests[0].yaw, 30.0);
            assert!(credit.upgrade().is_some());
            let retried: Vec<_> = hub
                .poll(ctx)
                .into_iter()
                .filter_map(|event| {
                    if let HostEvent::RawPayload(delivery) = event {
                        Some(delivery)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(retried.len(), 1);
            let crate::raw_payloads::Body::Payload(bytes) = &retried[0].incoming.body else {
                panic!("expected original payload")
            };
            assert_eq!(bytes.0, [1]);
            assert!(
                hub.drain_packets(ctx).is_empty(),
                "later movement waits for the retried root to finish"
            );
            drop(retried);
            hub.finish_raw_payload();
            let next: Vec<_> = hub
                .drain_packets(ctx)
                .into_iter()
                .filter_map(|event| {
                    if let HostEvent::RawPayload(delivery) = event {
                        Some(delivery)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(hub.guests[0].yaw, 30.0);
            assert_eq!(next.len(), 1);
            let crate::raw_payloads::Body::Payload(bytes) = &next[0].incoming.body else {
                panic!("expected second payload")
            };
            assert_eq!(bytes.0, [2]);
            assert!(credit.upgrade().is_none());
        });
    });
}
