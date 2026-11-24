//! The state of the recording player's session that is not in the world's chunks: the packets
//! that put a replay in the same place when its recording starts mid-session (the server sent
//! them long before). The game gathers the values from the live session; building the packets
//! lives here so the recording has one definition of what a snapshot says.
use horizon_protocol::{
    item::Stack,
    packets::play::{
        Clientbound as C, ClockState, GameEvent, SetExperience, SetHealth, SetHeldSlot,
        SetPlayerInventory, SetTime,
    },
};

/// `GameEvent` ids (`ClientboundGameEventPacket`) the weather uses.
const RAIN_LEVEL_CHANGE: u8 = 7;
const THUNDER_LEVEL_CHANGE: u8 = 8;

/// What the session holds for the recording player, as the client knows it.
#[derive(Clone, Debug, Default)]
pub struct SessionState {
    pub game_time: i64,
    /// The connection's world clocks (`ClientClockManager`): total ticks, partial tick and rate
    /// by `minecraft:world_clock` registry id. The level's day time is its default clock's total;
    /// empty before the server sent any.
    pub clocks: Vec<ClockState>,
    pub rain_level: f32,
    pub thunder_level: f32,
    /// Health, food and saturation.
    pub health: (f32, i32, f32),
    /// Experience progress, level and total points.
    pub experience: (f32, i32, i32),
    /// The selected hotbar slot, 0 to 8.
    pub selected_slot: usize,
    /// The non-empty slots of the player's inventory by `Inventory` index: 0 to 35 the hotbar
    /// then the main storage, 36 to 39 feet to head, 40 the off hand.
    pub inventory: Vec<(i32, Stack)>,
}

/// The packets that restore `state`, in an order that works for the client: time and weather
/// first, then what the player carries and their vitals.
pub fn packets(state: &SessionState) -> Vec<C> {
    let mut out = vec![C::SetTime(SetTime {
        game_time: state.game_time,
        clocks: state.clocks.clone(),
    })];
    // A level starts clear: only a storm needs saying.
    for (event, level) in [
        (RAIN_LEVEL_CHANGE, state.rain_level),
        (THUNDER_LEVEL_CHANGE, state.thunder_level),
    ] {
        if level > 0.0 {
            out.push(C::GameEvent(GameEvent {
                event,
                param: level,
            }));
        }
    }
    for (slot, stack) in &state.inventory {
        out.push(C::SetPlayerInventory(SetPlayerInventory {
            slot: *slot,
            item: Some(stack.clone()),
        }));
    }
    out.push(C::SetHeldSlot(SetHeldSlot {
        slot: state.selected_slot.min(8) as i32,
    }));
    let (health, food, saturation) = state.health;
    out.push(C::SetHealth(SetHealth {
        health,
        food,
        saturation,
    }));
    let (progress, level, total) = state.experience;
    out.push(C::SetExperience(SetExperience {
        progress,
        level,
        total,
    }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> SessionState {
        SessionState {
            game_time: 123_456,
            clocks: vec![
                ClockState {
                    clock: 0,
                    total_ticks: 18_000,
                    partial_tick: 0.25,
                    rate: 1.0,
                },
                ClockState {
                    clock: 1,
                    total_ticks: 6_000,
                    partial_tick: 0.0,
                    rate: 0.5,
                },
            ],
            rain_level: 0.0,
            thunder_level: 0.0,
            health: (13.5, 17, 2.0),
            experience: (0.5, 7, 120),
            selected_slot: 4,
            inventory: vec![(37, Stack::new(900, 1)), (40, Stack::new(901, 1))],
        }
    }

    #[test]
    fn a_clear_night_with_gear_is_restored_as_it_was() {
        let packets = packets(&state());
        let time = packets
            .iter()
            .find_map(|p| match p {
                C::SetTime(t) => Some(t),
                _ => None,
            })
            .expect("time");
        assert_eq!(time.game_time, 123_456);
        // Both clocks, with their own totals, partial ticks and rates.
        assert_eq!(time.clocks, state().clocks);
        assert_eq!(
            time.clocks
                .iter()
                .map(|c| (c.clock, c.total_ticks, c.partial_tick, c.rate))
                .collect::<Vec<_>>(),
            [(0, 18_000, 0.25, 1.0), (1, 6_000, 0.0, 0.5)]
        );
        // Nothing to say about weather while it is clear.
        assert!(!packets.iter().any(|p| matches!(p, C::GameEvent(_))));
        let carried: Vec<(i32, i32)> = packets
            .iter()
            .filter_map(|p| match p {
                C::SetPlayerInventory(s) => Some((s.slot, s.item.as_ref()?.item)),
                _ => None,
            })
            .collect();
        assert_eq!(carried, [(37, 900), (40, 901)]);
        assert!(packets.contains(&C::SetHeldSlot(SetHeldSlot { slot: 4 })));
        assert!(packets.contains(&C::SetHealth(SetHealth {
            health: 13.5,
            food: 17,
            saturation: 2.0
        })));
        assert!(packets.contains(&C::SetExperience(SetExperience {
            progress: 0.5,
            level: 7,
            total: 120
        })));
    }

    #[test]
    fn a_storm_is_sent_as_rain_and_thunder_levels() {
        let mut stormy = state();
        stormy.rain_level = 1.0;
        stormy.thunder_level = 0.75;
        let events: Vec<(u8, f32)> = packets(&stormy)
            .iter()
            .filter_map(|p| match p {
                C::GameEvent(e) => Some((e.event, e.param)),
                _ => None,
            })
            .collect();
        assert_eq!(events, [(7, 1.0), (8, 0.75)]);
    }

    #[test]
    fn a_level_without_a_clock_sends_no_day_time() {
        let mut unclocked = state();
        unclocked.clocks.clear();
        let packets = packets(&unclocked);
        assert!(
            packets.iter().any(
                |p| matches!(p, C::SetTime(t) if t.clocks.is_empty() && t.game_time == 123_456)
            )
        );
    }

    #[test]
    fn every_packet_survives_the_wire() {
        for packet in packets(&state()) {
            let encoded = packet.encode();
            let mut reader = horizon_protocol::buf::Reader::new(&encoded);
            let id = reader.varint().unwrap();
            assert_eq!(C::decode(id, reader.rest()).unwrap(), packet);
        }
    }
}
