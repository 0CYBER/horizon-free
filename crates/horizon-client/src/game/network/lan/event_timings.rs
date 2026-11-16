//! Attribution for slow steps made of many short events: each event under the slow-step threshold
//! has no note of its own, so a whole second of 5 ms `UseItemOn`s would name nobody (#10255).
//!
//! Events are grouped by packet kind, as a freeze is usually one kind of packet from many players.
//! A group reports only the time of its short events (the slow ones are noted on their own), and
//! names the player who spent the most of it. Kinds are packet names and players are the
//! server's guests, so the groups stay as few as those, and only while timing is recorded.

use super::LanHost;
use horizon_client::lan::hub::HostEvent;
use std::{sync::Arc, time::Instant};

pub(super) struct Burst<W, D> {
    pub kind: &'static str,
    /// What every event of the group was about, when it was the same for all of them.
    pub detail: Option<D>,
    /// The short events and their total time.
    pub count: u32,
    pub ms: f32,
    /// The player with the most of that time: who, how many events, how long.
    pub top: Option<(W, u32, f32)>,
}

struct Group<W, D> {
    kind: &'static str,
    detail: Option<D>,
    mixed: bool,
    count: u32,
    ms: f32,
    players: Vec<(W, u32, f32)>,
}

pub(super) struct EventTimings<W, D> {
    threshold: f32,
    groups: Vec<Group<W, D>>,
}

impl<W: PartialEq, D: PartialEq> EventTimings<W, D> {
    /// `threshold`: the time from which an event is noted on its own.
    pub fn new(threshold: f32) -> Self {
        Self {
            threshold,
            groups: Vec::new(),
        }
    }

    pub fn add(&mut self, kind: &'static str, who: W, detail: Option<D>, ms: f32) {
        if ms >= self.threshold {
            return;
        }
        let i = match self.groups.iter().position(|g| g.kind == kind) {
            Some(i) => i,
            None => {
                self.groups.push(Group {
                    kind,
                    detail: None,
                    mixed: false,
                    count: 0,
                    ms: 0.0,
                    players: Vec::new(),
                });
                self.groups.len() - 1
            }
        };
        let g = &mut self.groups[i];
        // A burst can visit different blocks or change the held item: then it names no target.
        if g.count == 0 {
            g.detail = detail;
        } else if !g.mixed && g.detail != detail {
            g.detail = None;
            g.mixed = true;
        }
        g.count = g.count.saturating_add(1);
        g.ms += ms;
        match g.players.iter_mut().find(|p| p.0 == who) {
            Some(p) => {
                p.1 = p.1.saturating_add(1);
                p.2 += ms;
            }
            None => g.players.push((who, 1, ms)),
        }
    }

    /// The groups whose short events add up to a slow step.
    pub fn bursts(self) -> impl Iterator<Item = Burst<W, D>> {
        let threshold = self.threshold;
        self.groups
            .into_iter()
            .filter(move |g| g.ms >= threshold)
            .map(|g| Burst {
                kind: g.kind,
                detail: g.detail,
                count: g.count,
                ms: g.ms,
                top: g
                    .players
                    .into_iter()
                    .reduce(|a, b| if b.2 > a.2 { b } else { a }),
            })
    }
}

/// What a hub event is, whose connection, and the name of a player who left (no longer in the
/// hub once it is handled): for a slow tick's notes.
pub(super) type EventKind = (&'static str, Option<u32>, Option<String>);

pub(super) fn event_kind(event: &HostEvent) -> EventKind {
    match event {
        HostEvent::Joined(c) => ("join", Some(*c), None),
        HostEvent::Left(g) => ("leave", None, Some(g.name.clone())),
        HostEvent::Packet(c, p) => (p.name(), Some(*c), None),
        HostEvent::InputCredits(_) => ("input credits", None, None),
        HostEvent::RawPayload(_) => ("raw payload", None, None),
        HostEvent::ClientInformation(c, _, _) => ("client settings", Some(*c), None),
        HostEvent::ChatSubmitted { .. } => ("chat submitted", None, None),
        HostEvent::ChatSessionStarted(_) => ("chat session", None, None),
        HostEvent::Jump { conn, .. } => ("jump", Some(*conn), None),
        HostEvent::PlayerMove(c) => ("player move", Some(*c), None),
        HostEvent::PlayerLoaded(c) => ("loaded", Some(*c), None),
        HostEvent::RecipeBookSettings { conn, .. } => ("recipe book", Some(*conn), None),
        HostEvent::Sneak { conn, .. } => ("sneak", Some(*conn), None),
        HostEvent::Input { conn, .. } => ("input", Some(*conn), None),
        HostEvent::Kicked { name, .. } => ("kick", None, Some(name.clone())),
        HostEvent::Flight { conn, .. } => ("flight", Some(*conn), None),
        HostEvent::Chat { .. } => ("chat line", None, None),
        HostEvent::PlayerChat { .. } => ("chat", None, None),
        HostEvent::ChatFiltered { name, .. } => ("filtered chat", None, Some(name.clone())),
        HostEvent::Warn(_) => ("warning", None, None),
        HostEvent::Error(_) => ("error", None, None),
        HostEvent::Ban { name, .. } => ("ban", None, Some(name.clone())),
    }
}

/// Notes how long the event that started at `started` took (`spans::note`), with what it
/// was about when `Game::event_detail` said.
pub(super) fn note_event(
    host: &LanHost,
    started: Option<(Instant, EventKind, Option<UseDetail>)>,
    timings: &mut EventTimings<(Option<u32>, Option<String>), UseDetail>,
) {
    let Some((started, kind, detail)) = started else {
        return;
    };
    let ms = started.elapsed().as_secs_f32() * 1000.0;
    horizon_client::spans::note(ms, || describe_event(host, &kind, detail.as_ref()));
    let (name, conn, player) = kind;
    timings.add(name, (conn, player), detail, ms);
}

/// The player an event came from: its name, else its connection.
fn who(host: &LanHost, conn: Option<u32>, name: Option<String>) -> Option<String> {
    name.or_else(|| conn.and_then(|c| host.hub.guest(c)).map(|g| g.name.clone()))
        .or_else(|| conn.map(|c| format!("connection {c}")))
}

/// A burst's note: `213 x UseItemOn under 10 ms each (…); most from Steve: 140 of them, 710.0 ms`.
pub(super) fn describe_burst(
    host: &LanHost,
    burst: &Burst<(Option<u32>, Option<String>), UseDetail>,
) -> String {
    let mut line = format!(
        "{} x {} under {} ms each",
        burst.count,
        burst.kind,
        horizon_client::spans::NOTE_MS
    );
    if let Some(d) = &burst.detail {
        line.push_str(&format!(" ({d})"));
    }
    if let Some(((conn, name), count, ms)) = &burst.top
        && let Some(who) = who(host, *conn, name.clone())
    {
        line.push_str(&format!("; most from {who}: {count} of them, {ms:.1} ms"));
    }
    line
}

pub(super) fn describe_event(
    host: &LanHost,
    (kind, conn, name): &EventKind,
    detail: Option<&UseDetail>,
) -> String {
    let line = match who(host, *conn, name.clone()) {
        Some(who) => format!("{kind} of {who}"),
        None => (*kind).to_owned(),
    };
    match detail {
        Some(d) => format!("{line} ({d})"),
        None => line,
    }
}

/// What a use packet was about, taken before it ran (the use may consume the item or change
/// the block): the item in the hand it names and, on a block, that block, where, and the face.
#[derive(PartialEq)]
pub(super) struct UseDetail {
    pub(super) item: Option<Arc<str>>,
    pub(super) block: Option<(Arc<str>, [i32; 3], i32)>,
}

impl std::fmt::Display for UseDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.item.as_deref().unwrap_or("empty hand"))?;
        if let Some((block, [x, y, z], face)) = &self.block {
            let face = ["down", "up", "north", "south", "west", "east"]
                .get(*face as usize)
                .copied()
                .unwrap_or("?");
            write!(f, " on {block} at {x} {y} {z} {face}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bursts(
        timings: EventTimings<&'static str, &'static str>,
    ) -> Vec<Burst<&'static str, &'static str>> {
        timings.bursts().collect()
    }

    #[test]
    fn one_short_event_from_each_of_many_players_is_still_named() {
        let mut timings = EventTimings::new(10.0);
        let players: Vec<String> = (0..69).map(|i| format!("p{i}")).collect();
        for (i, name) in players.iter().enumerate() {
            let ms = if i == 40 { 9.0 } else { 6.0 };
            timings.add("UseItemOn", name.as_str(), None::<()>, ms);
        }
        let b: Vec<_> = timings.bursts().collect();
        assert_eq!(b.len(), 1);
        assert_eq!(
            (b[0].kind, b[0].count, b[0].ms),
            ("UseItemOn", 69, 68.0 * 6.0 + 9.0)
        );
        assert_eq!(b[0].top, Some(("p40", 1, 9.0)));
    }

    #[test]
    fn a_late_burst_is_named_however_many_other_groups_came_first() {
        let mut timings = EventTimings::new(10.0);
        for i in 0..100 {
            let name: &'static str = Box::leak(format!("p{i}").into_boxed_str());
            timings.add("MovePlayerPos", name, None, 0.05);
            timings.add("Chat", name, None, 0.01);
        }
        for i in 0..200 {
            timings.add(
                "UseItemOn",
                if i % 3 == 0 { "Alex" } else { "Steve" },
                Some("chest"),
                5.0,
            );
        }
        let b = bursts(timings);
        assert_eq!(
            b.len(),
            1,
            "movement (5 ms) and chat (1 ms) stay under 10 ms"
        );
        assert_eq!((b[0].kind, b[0].count, b[0].ms), ("UseItemOn", 200, 1000.0));
        assert_eq!(b[0].detail, Some("chest"));
        assert_eq!(b[0].top, Some(("Steve", 133, 665.0)));
    }

    #[test]
    fn events_noted_on_their_own_are_left_out_of_the_burst() {
        let mut timings = EventTimings::new(10.0);
        timings.add("UseItemOn", "Steve", None::<()>, 15.0);
        for _ in 0..200 {
            timings.add("UseItemOn", "Alex", None, 5.0);
        }
        let b: Vec<_> = timings.bursts().collect();
        assert_eq!((b[0].count, b[0].ms), (200, 1000.0));
        assert_eq!(b[0].top, Some(("Alex", 200, 1000.0)));
    }

    #[test]
    fn short_events_that_stay_short_together_report_nothing() {
        let mut timings = EventTimings::new(10.0);
        timings.add("UseItemOn", "Steve", None::<()>, 4.0);
        timings.add("UseItemOn", "Alex", None, 4.0);
        timings.add("UseItem", "Steve", None, 9.0);
        assert!(timings.bursts().next().is_none());
    }

    #[test]
    fn mixed_targets_name_none_even_when_the_first_comes_back() {
        let mut timings = EventTimings::new(10.0);
        for detail in ["chest", "sign", "chest"] {
            timings.add("UseItemOn", "Steve", Some(detail), 6.0);
        }
        let b = bursts(timings);
        assert_eq!(b[0].detail, None);
        assert_eq!(b[0].ms, 18.0);
    }
}
