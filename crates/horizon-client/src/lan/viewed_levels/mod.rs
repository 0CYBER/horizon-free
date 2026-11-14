//! Viewed levels, server side (#3716): levels besides their own that players' clients keep, so
//! that the server can send them those levels' chunks and updates on `horizon:levels`
//! (`horizon_protocol::viewed_levels`).
//!
//! A mod asks for areas of chunks for one player in any level (`Hub::set_viewed_areas`). They hold
//! their own tickets (`ticket_levels::Sources::viewed`) and loading squares (`Streaming.extra`),
//! never the forced chunks' or the mods' chunk tickets, and they live apart from what makes a
//! player count in a level (`Guest::placed_in`, `Guest::watches_level`): an area is not a player
//! for spawning, sleeping, raids or any other rule. A client whose capability answer allows it is
//! sent the areas of the levels it is not in, each packet wrapped with its level (`send`), and
//! the entities there (`entities`). A travel into a level its client views swaps that level with
//! the one it leaves (`switch`). From revision 2 their sounds go too (`sounds`).
//!
//! With no area on the server, a level pass costs one integer compare ([`ViewedLevels::active`]).
pub mod areas;
pub mod capability;
pub(super) mod entities;
pub(super) mod send;
mod sounds;
mod switch;
#[cfg(test)]
mod tests;
pub mod view;

use super::{guest::Guest, hub::Hub};
use crate::ticket_levels::ViewedTicket;
use areas::{Areas, ChunkArea, ChunkAreaMode, ChunkAreaRefusal, ViewedCaps};
use horizon_protocol::viewed_levels::{Answer, Level};
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use view::View;

/// The server's part: the caps, and how many players have areas or viewed levels left.
#[derive(Default)]
pub struct ViewedLevels {
    pub caps: ViewedCaps,
    active: Arc<AtomicUsize>,
}

impl ViewedLevels {
    /// Whether any player has an area or a viewed level left: while none has, nothing of this
    /// runs.
    pub fn active(&self) -> bool {
        self.active.load(Ordering::Relaxed) != 0
    }
}

/// A player counted in [`ViewedLevels::active`] while it holds one; dropped with its guest.
struct Counted(Arc<AtomicUsize>);

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// One player's areas and the levels its client views (`Guest.viewed`). Not saved: it goes with
/// the player when it disconnects.
#[derive(Default)]
pub struct Viewed {
    areas: Areas,
    /// The `(loaded, ticking)` chunks the areas hold in each level they name.
    counts: BTreeMap<String, (u32, u32)>,
    /// The viewed levels open on the client, by dimension.
    pub(super) views: BTreeMap<String, View>,
    /// The levels its client was told at Login: before revision 4 a viewed level is named by its
    /// index there. A level opened since (a runtime level) is not among them, and one closed
    /// since keeps its place.
    login_levels: Vec<String>,
    /// From revision 4, the epochs its `open`s and travel swaps took: the next is this one. Past
    /// `i32::MAX` they are spent, and nothing more opens (`horizon_protocol::viewed_levels`).
    epochs: i64,
    counted: Option<Counted>,
}

impl Viewed {
    /// Whether the areas name `dimension`, or its client holds chunks of it still.
    pub fn in_level(&self, dimension: &str) -> bool {
        self.counts.contains_key(dimension) || self.views.contains_key(dimension)
    }
    /// The open view of `dimension`.
    pub fn view(&self, dimension: &str) -> Option<&View> {
        self.views.get(dimension)
    }
    /// Every mod's areas in `dimension`.
    fn areas_in<'a>(&'a self, dimension: &'a str) -> impl Iterator<Item = &'a ChunkArea> + 'a {
        self.areas
            .values()
            .flatten()
            .filter(move |a| a.level == dimension)
    }
    /// The Login packet's `levels`, as its client was sent them.
    pub fn logged_in_with(&mut self, levels: Vec<String>) {
        self.login_levels = levels;
    }
    /// `dimension`'s index in the levels its client was told at Login: none for a level opened
    /// since.
    fn login_index(&self, dimension: &str) -> Option<i32> {
        let i = self.login_levels.iter().position(|l| l == dimension)?;
        i32::try_from(i).ok()
    }
    /// Whether no epoch is left (`horizon_protocol::viewed_levels`).
    pub fn epochs_spent(&self) -> bool {
        i32::try_from(self.epochs).is_err()
    }
    /// The next epoch, taken; none once they are spent.
    fn take_epoch(&mut self) -> Option<i32> {
        let epoch = i32::try_from(self.epochs).ok()?;
        self.epochs += 1;
        Some(epoch)
    }
    /// What the client of `answer` is to name `dimension` by in a new view
    /// (`horizon_protocol::viewed_levels`): from revision 4 the first free slot below its cap,
    /// with the next epoch; before it, its index at Login. None when it cannot be named: a level
    /// opened since Login on an older client, no free slot, or the epochs spent.
    fn name_new(&mut self, dimension: &str, answer: Answer) -> Option<Level> {
        if !answer.bindings() {
            return self.login_index(dimension).map(Level::Index);
        }
        let taken = |slot: i32| {
            self.views
                .values()
                .any(|view| matches!(view.level, Level::Bound { slot: s, .. } if s == slot))
        };
        let slot = (0..i32::from(answer.max_levels)).find(|&slot| !taken(slot))?;
        let epoch = self.take_epoch()?;
        Some(Level::Bound { slot, epoch })
    }
    /// What the level `left` is named by once a travel swaps it with the view of the level
    /// entered, named `entered` (`switch`): from revision 4 the slot of the level entered, with
    /// the next epoch; before it, its index at Login. None when the swap cannot name it.
    fn name_left(&mut self, left: &str, entered: Level, answer: Answer) -> Option<Level> {
        match entered {
            Level::Bound { slot, .. } if answer.bindings() => {
                let epoch = self.take_epoch()?;
                Some(Level::Bound { slot, epoch })
            }
            _ => self.login_index(left).map(Level::Index),
        }
    }
    /// The client dropped every viewed world (a Respawn that is no travel into one of them,
    /// `switch`): they are forgotten without `close`, and opened again for what the areas still
    /// ask.
    pub fn forget_views(&mut self) {
        self.views.clear();
    }
    /// Counts the player in `active` while it has areas or views, and stops once it has neither.
    fn recount(&mut self, active: &Arc<AtomicUsize>) {
        let wanted = !self.areas.is_empty() || !self.views.is_empty();
        if wanted && self.counted.is_none() {
            active.fetch_add(1, Ordering::Relaxed);
            self.counted = Some(Counted(active.clone()));
        } else if !wanted {
            self.counted = None;
        }
    }
}

impl Hub {
    /// The engine's entry point for a mod's areas for the player `uuid`, in place of what `owner`
    /// asked before; none withdraws them. The chunks no area covers any more are forgotten
    /// [`view::LINGER`] ticks later. A call that breaks a limit changes nothing and says which.
    pub fn set_viewed_areas(
        &mut self,
        uuid: u128,
        owner: &str,
        areas: Vec<ChunkArea>,
    ) -> Result<(), ChunkAreaRefusal> {
        let Some(i) = self
            .guests
            .iter()
            .position(|g| g.uuid == uuid && g.participates_online())
        else {
            return Err(ChunkAreaRefusal::NotOnline);
        };
        let others = |level: &str| {
            self.guests
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .filter_map(|(_, g)| g.viewed.counts.get(level))
                .fold((0, 0), |(l, t), &(gl, gt)| (l + gl, t + gt))
        };
        let g = &self.guests[i];
        let counts = areas::check(
            &g.viewed.areas,
            owner,
            &areas,
            &g.dimension,
            g.sender.viewed_levels.max_levels,
            &self.viewed.caps.for_client(g.sender.viewed_levels),
            others,
        )?;
        let active = self.viewed.active.clone();
        self.guests.update(i, |g| {
            let v = &mut g.viewed;
            if areas.is_empty() {
                v.areas.remove(owner);
            } else {
                v.areas.insert(owner.to_owned(), areas);
            }
            v.counts = counts;
            for view in v.views.values_mut() {
                view.dirty = true;
            }
            v.recount(&active);
        });
        Ok(())
    }

    /// What `owner` asked for the player `uuid`; none for no such player.
    pub fn viewed_areas(&self, uuid: u128, owner: &str) -> Vec<ChunkArea> {
        self.guests
            .iter()
            .find(|g| g.uuid == uuid)
            .and_then(|g| g.viewed.areas.get(owner).cloned())
            .unwrap_or_default()
    }

    /// Every player's areas in `dimension`: none, without looking at a player, while no player
    /// has any.
    fn viewed_areas_in<'a>(&'a self, dimension: &'a str) -> impl Iterator<Item = &'a ChunkArea> {
        let guests: &[Guest] = if self.viewed.active() {
            self.guests.as_slice()
        } else {
            &[]
        };
        guests
            .iter()
            .flat_map(move |g| g.viewed.areas_in(dimension))
    }

    /// The squares `Streaming.extra` keeps loaded in `dimension` for the areas there
    /// (`lan_update_streaming`).
    pub fn viewed_extra(&self, dimension: &str) -> Vec<((i32, i32), i32)> {
        self.viewed_areas_in(dimension)
            .map(ChunkArea::extra)
            .collect()
    }

    /// The areas' tickets in `dimension`.
    pub fn viewed_tickets(&self, dimension: &str) -> Vec<ViewedTicket> {
        self.viewed_areas_in(dimension)
            .map(|a| ViewedTicket {
                center: a.center,
                radius: a.radius as i32,
                ticking: a.mode == ChunkAreaMode::Ticking,
            })
            .collect()
    }

    /// What the areas ask of `dimension`: None for no area there, else whether one ticks (the
    /// level then ticks, `levels_to_tick`; otherwise it gets a view pass).
    pub fn viewed_level(&self, dimension: &str) -> Option<bool> {
        let mut any = false;
        for a in self.viewed_areas_in(dimension) {
            if a.mode == ChunkAreaMode::Ticking {
                return Some(true);
            }
            any = true;
        }
        any.then_some(false)
    }

    /// Whether a player's client still holds chunks of `dimension` as a viewed level: after its
    /// last area there is withdrawn, until the forgets and `close` went out. It holds no ticket.
    pub fn viewed_open(&self, dimension: &str) -> bool {
        self.viewed.active()
            && self
                .guests
                .iter()
                .any(|g| g.viewed.views.contains_key(dimension))
    }

    /// Whether a player's client views the level in place.
    pub fn viewed_here(&self) -> bool {
        self.viewed.active()
            && self
                .guests
                .iter()
                .any(|g| g.views_level(&self.config.dimension))
    }

    /// The level `dimension` closed (a runtime level unloaded or deleted): each client that
    /// views it is told `close`, so that a level opened again under its id is sent afresh. The
    /// areas there stay, and view it again once it is open.
    pub fn close_views_of(&mut self, dimension: &str) {
        if !self.viewed.active() {
            return;
        }
        let active = self.viewed.active.clone();
        for i in 0..self.guests.len() {
            self.guests.update(i, |g| {
                if let Some(view) = g.viewed.views.remove(dimension) {
                    g.sender
                        .send(horizon_protocol::viewed_levels::close(view.level));
                }
                g.viewed.recount(&active);
            });
        }
    }

    /// The view of the level in place `g`'s client holds, if it holds one.
    pub(super) fn view_here<'g>(&self, g: &'g Guest) -> Option<&'g View> {
        if !g.views_level(&self.config.dimension) {
            return None;
        }
        g.viewed.view(&self.config.dimension)
    }
}
