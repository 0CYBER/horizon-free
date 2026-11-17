#![cfg(feature = "state-digest")]
//! Per-level-tick state digest to `HORIZON_STATE_DIGEST` (`_DETAIL=1` adds per-entity lines).
use super::{
    Dim, FallingBlock, Game, boats,
    entity::{Entity as _, EntityHandle, entity_lists_of, with_entity},
    fireworks, general, minecarts, tnt, trident,
};
use horizon_client::join_seq::JoinSeq;
use horizon_core::{geometry::Vec3, random::LegacyRandom};
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    io::Write,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

struct Sink {
    out: std::io::BufWriter<std::fs::File>,
    detail: bool,
    nanos: u128,
    lines: u64,
}

fn sink() -> Option<&'static Mutex<Sink>> {
    static SINK: OnceLock<Option<Mutex<Sink>>> = OnceLock::new();
    SINK.get_or_init(|| {
        let path = std::env::var_os("HORIZON_STATE_DIGEST")?;
        let file = std::fs::File::create(&path).ok()?;
        Some(Mutex::new(Sink {
            out: std::io::BufWriter::new(file),
            detail: std::env::var("HORIZON_STATE_DIGEST_DETAIL").is_ok_and(|v| v == "1"),
            nanos: 0,
            lines: 0,
        }))
    })
    .as_ref()
}

/// `JoinSeq::upcoming()` at `digest-sync`: join numbers are hashed relative to it, as the counter is process-wide.
static JOIN_BASE: AtomicU64 = AtomicU64::new(0);

fn relative(seq: u64) -> u64 {
    seq.wrapping_sub(JOIN_BASE.load(Ordering::Relaxed))
}

fn hasher() -> DefaultHasher {
    DefaultHasher::new()
}

fn vec3(h: &mut impl Hasher, v: Vec3) {
    v.x.to_bits().hash(h);
    v.y.to_bits().hash(h);
    v.z.to_bits().hash(h);
}

fn random_words(r: &LegacyRandom) -> (u64, Option<u64>) {
    (r.state(), r.cached_gaussian().map(f64::to_bits))
}

/// Health, own random and kind fields the `Entity` trait does not expose.
trait DigestParts {
    fn health_bits(&self) -> Option<u64> {
        None
    }
    fn own_random(&self) -> Option<&LegacyRandom> {
        None
    }
    fn extra(&self) -> i64 {
        0
    }
}

impl DigestParts for horizon_client::mobs::Mob {
    fn health_bits(&self) -> Option<u64> {
        Some(u64::from(self.health.to_bits()))
    }
    fn own_random(&self) -> Option<&LegacyRandom> {
        Some(&self.random)
    }
}
impl DigestParts for horizon_client::entities::ItemEntity {
    fn health_bits(&self) -> Option<u64> {
        Some(self.health as u64)
    }
    fn own_random(&self) -> Option<&LegacyRandom> {
        Some(&self.random)
    }
    fn extra(&self) -> i64 {
        (i64::from(self.stack.count) << 32) | i64::from(self.age as u32)
    }
}
impl DigestParts for boats::Boat {
    fn own_random(&self) -> Option<&LegacyRandom> {
        Some(&self.random)
    }
}
impl DigestParts for minecarts::Minecart {
    fn health_bits(&self) -> Option<u64> {
        Some(u64::from(self.damage.to_bits()))
    }
    fn own_random(&self) -> Option<&LegacyRandom> {
        Some(&self.random)
    }
}
impl DigestParts for horizon_client::entities::ArmorStand {
    fn health_bits(&self) -> Option<u64> {
        Some(u64::from(self.health.to_bits()))
    }
}
impl DigestParts for tnt::PrimedTnt {
    fn extra(&self) -> i64 {
        i64::from(self.fuse)
    }
}
impl DigestParts for FallingBlock {}
impl DigestParts for horizon_client::entities::ExperienceOrb {
    fn health_bits(&self) -> Option<u64> {
        Some(self.health as u64)
    }
    fn extra(&self) -> i64 {
        i64::from(self.value)
    }
}
impl DigestParts for horizon_client::projectiles::Projectile {
    fn own_random(&self) -> Option<&LegacyRandom> {
        self.arrow.as_ref().map(|_| &*self.random.0)
    }
}
impl DigestParts for fireworks::Rocket {}
impl DigestParts for general::GeneralRow {}
impl DigestParts for trident::ThrownTrident {
    fn own_random(&self) -> Option<&LegacyRandom> {
        Some(&self.random)
    }
}

impl Game<'_> {
    /// The end of one level's tick: append its digest line.
    pub(super) fn state_digest_tick(&mut self, dim: Dim, running: Option<bool>) {
        let Some(sink) = sink() else {
            return;
        };
        let start = Instant::now();
        let mut sink = sink.lock().unwrap();
        let detail = sink.detail;
        let mut details = String::new();

        let (mut entities, mut randoms) = (hasher(), hasher());
        let (mut count, mut with_random) = (0u64, 0u64);
        let mut visit = |handle: EntityHandle, seq: JoinSeq| {
            with_entity!(self, handle, e => {
                let mut one = hasher();
                std::mem::discriminant(&handle).hash(&mut one);
                e.native_type().hash(&mut one);
                e.id().hash(&mut one);
                relative(seq.get()).hash(&mut one);
                vec3(&mut one, e.position());
                vec3(&mut one, e.velocity());
                let [yaw, pitch] = e.rotation();
                yaw.to_bits().hash(&mut one);
                pitch.to_bits().hash(&mut one);
                e.health_bits().hash(&mut one);
                e.is_removed().hash(&mut one);
                e.extra().hash(&mut one);
                let one = one.finish();
                one.hash(&mut entities);
                count += 1;
                let random = e.own_random().map(random_words);
                if let Some(words) = random {
                    e.id().hash(&mut randoms);
                    words.hash(&mut randoms);
                    with_random += 1;
                }
                if detail {
                    let (p, v) = (e.position(), e.velocity());
                    details.push_str(&format!(
                        "  e {} {} id={} join={} pos={:?} vel={:?} rot={:?} health={:?} removed={} extra={} random={:?}\n",
                        count - 1,
                        e.native_type(),
                        e.id(),
                        relative(seq.get()),
                        [p.x, p.y, p.z],
                        [v.x, v.y, v.z],
                        e.rotation(),
                        e.health_bits(),
                        e.is_removed(),
                        e.extra(),
                        random,
                    ));
                }
            })
        };
        let walk = entity_lists_of!(self).walk_in_join_order(&mut visit);
        let walk_error = walk.err().map(|e| format!("{e:?}"));
        if walk_error.is_some() {
            // The join-order walk refused the lists; hash them list by list instead.
            for handle in self.entity_handles() {
                visit(handle, JoinSeq::UNJOINED);
            }
        }

        let mut block_entities = hasher();
        let tickers = self.world.block_entities.ticker_slots();
        let mut be_count = 0u64;
        for &(pos, slot) in tickers.iter() {
            let Some(e) = self.world.block_entities.get(&pos) else {
                continue;
            };
            let name = self.data_block_name([pos.0, pos.1, pos.2]);
            let tag = horizon_client::save::block_entity_encode::encode(pos, e, &name);
            let mut one = hasher();
            pos.hash(&mut one);
            slot.hash(&mut one);
            name.hash(&mut one);
            tag.hash(&mut one);
            let one = one.finish();
            one.hash(&mut block_entities);
            be_count += 1;
            if detail {
                details.push_str(&format!("  be {pos:?} slot={slot} {name} {one:016x}\n"));
            }
        }

        let level_random = random_words(&self.world.random);
        let next_join = relative(JoinSeq::upcoming().get());
        let mut ticks = hasher();
        let block_ticks = sorted_ticks(&self.block_ticks.blocks);
        let fluid_ticks = sorted_ticks(&self.block_ticks.fluids);
        block_ticks.hash(&mut ticks);
        fluid_ticks.hash(&mut ticks);

        let line = format!(
            "tick={} dim={} run={} entities={count}:{:016x} randoms={with_random}:{:016x} \
             block_entities={be_count}:{:016x} level_random={:012x}/{:?}/{} counters={}/{} \
             scheduled={}/{}:{:016x}{}\n",
            self.world.game_time,
            dim.id(),
            match running {
                Some(true) => "1",
                Some(false) => "0",
                None => "-",
            },
            entities.finish(),
            randoms.finish(),
            block_entities.finish(),
            level_random.0,
            level_random.1,
            self.block_ticks.random_tick_value,
            self.next_entity,
            next_join,
            block_ticks.len(),
            fluid_ticks.len(),
            ticks.finish(),
            walk_error.map_or(String::new(), |e| format!(" walk_error={e}")),
        );
        sink.nanos += start.elapsed().as_nanos();
        sink.lines += 1;
        let (nanos, lines) = (sink.nanos, sink.lines);
        let _ = sink.out.write_all(line.as_bytes());
        let _ = sink.out.write_all(details.as_bytes());
        let _ = writeln!(sink.out, "  cost_total_ns={nanos} lines={lines}");
        let _ = sink.out.flush();
    }

    /// `digest-sync U E S`: the command harness's `sync_randoms`, from a dedicated server's console.
    pub(super) fn state_digest_sync(&mut self, args: &[&str]) -> Result<(), String> {
        let [u, e, s] = args else {
            return Err("digest-sync UNIQUIFIER ENTITY_ID LEVEL_SEED".into());
        };
        let num = |v: &str| v.parse::<i64>().map_err(|err| format!("{v}: {err}"));
        let (u, e, s) = (num(u)?, num(e)?, num(s)?);
        horizon_client::random_source::set_thread_uniquifier(u)?;
        self.next_entity = u32::try_from(e).map_err(|err| err.to_string())?;
        horizon_core::random::RandomSource::set_seed(&mut self.world.random, s);
        JOIN_BASE.store(JoinSeq::upcoming().get(), Ordering::Relaxed);
        Ok(())
    }
}

/// Every queued tick of `ticks`, in a fixed order.
fn sorted_ticks(ticks: &horizon_core::ticks::LevelTicks) -> Vec<(i64, u32, [i32; 3], i8, i64)> {
    let mut all: Vec<_> = ticks
        .all_queued()
        .map(|t| {
            (
                t.trigger_tick,
                t.kind.0,
                [t.pos.x, t.pos.y, t.pos.z],
                t.priority as i8,
                t.sub_tick_order,
            )
        })
        .collect();
    all.sort_unstable();
    all
}
