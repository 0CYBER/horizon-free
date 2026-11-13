//! The two halves of the parallel natural spawning pass (docs/parallel-natural-spawning.md):
//! a [`Lane`] proposes the spawns of one chunk after another against the caps as the pass found
//! them, and [`commit`] replays the proposals in the pass's chunk order against the caps as the
//! spawns before them have raised them.
use super::*;

/// One mob of a pack a lane proposes: what the commit checks again, and the mob made for it.
#[derive(Debug)]
pub struct Member<M> {
    pub entity: String,
    pub species: Species,
    /// The category of the mob's type (what `after_spawn` counts it under).
    pub category: Category,
    /// `blockPosition()`.
    pub block: [i32; 3],
    pub bounds: Aabb,
    pub made: M,
}
/// What one `spawnCategoryForChunk` of a chunk proposed, in order: a later member stands on the
/// earlier ones having spawned, so a rejected member ends the pack.
#[derive(Debug)]
pub struct Pack<M> {
    pub category: Category,
    pub chunk: (i32, i32),
    pub members: Vec<Member<M>>,
}

/// A lane's spawn state: a copy of the pass's start, so every chunk it proposes for sees the caps
/// as they stood then, whichever chunks the lane had before it.
pub struct Lane<'b> {
    base: &'b SpawnState,
    state: SpawnState,
}
impl<'b> Lane<'b> {
    pub fn new(base: &'b SpawnState) -> Self {
        Self {
            base,
            state: base.clone(),
        }
    }
    /// `spawnForChunk` for the categories already filtered: each category's pack the chunk
    /// proposes. `make` creates the mob for a request and its box, or refuses it.
    pub fn chunk<L: NaturalLevel, R: RandomSource, M>(
        &mut self,
        l: &L,
        random: &mut R,
        categories: &[Category],
        chunk: (i32, i32),
        make: &mut dyn FnMut(&L, &SpawnRequest) -> Option<(Spawned, M)>,
    ) -> Vec<Pack<M>> {
        let mut packs = Vec::new();
        for &category in categories {
            if !self.state.can_spawn_local(category, chunk.0, chunk.1) {
                continue;
            }
            let mut members = Vec::new();
            let mut spawn = |l: &L, r: &SpawnRequest| {
                let kind = l.mob_type(r.entity)?;
                let (spawned, made) = make(l, r)?;
                members.push(Member {
                    entity: r.entity.to_owned(),
                    species: r.species,
                    category: kind.3,
                    block: [
                        r.position[0].floor() as i32,
                        r.position[1].floor() as i32,
                        r.position[2].floor() as i32,
                    ],
                    bounds: spawned.bounds,
                    made,
                });
                Some(spawned)
            };
            spawn_category_for_chunk(
                l,
                random,
                &mut self.state,
                category,
                chunk,
                &mut spawn,
                &mut |_| {},
            );
            if !members.is_empty() {
                packs.push(Pack {
                    category,
                    chunk,
                    members,
                });
            }
        }
        if !packs.is_empty() {
            // The proposals raised this copy's counts; the next chunk starts from the pass's.
            self.state = self.base.clone();
        }
        packs
    }
}

/// Replays a chunk's packs on the pass's own state, `accepted` holding the boxes of the mobs the
/// pass has taken so far: a pack whose category the chunk's players have filled since is dropped,
/// and so is a pack from the first member whose spawn charge no longer fits or whose box meets
/// an accepted mob's. An accepted member counts as `afterSpawn` does and goes to `accept`.
pub fn commit<L: NaturalLevel, M>(
    state: &mut SpawnState,
    l: &L,
    packs: Vec<Pack<M>>,
    accepted: &mut Vec<Aabb>,
    mut accept: impl FnMut(M),
) {
    for pack in packs {
        if !state.can_spawn_local(pack.category, pack.chunk.0, pack.chunk.1) {
            continue;
        }
        for m in pack.members {
            // `checkSpawnObstruction` of the species that look for a mob in the way.
            let looks = !matches!(obstruction(m.species), Obstruction::NoLiquid);
            if !state.can_spawn(l, &m.entity, m.block)
                || (looks && accepted.iter().any(|b| b.intersects(m.bounds)))
            {
                break;
            }
            state.after_spawn(l, &m.entity, m.category, m.block);
            attempt_count(1);
            accepted.push(m.bounds);
            accept(m.made);
        }
    }
}
