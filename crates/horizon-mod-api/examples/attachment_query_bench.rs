//! Production query/index benchmark. Run optimized, never as a timing assertion in CI:
//! cargo run -p horizon-mod-api --profile fast --example attachment_query_bench
//! The host adapter supplies real attachment maps/indexes, not a second query engine.
use horizon_mod_runtime::attachments::{self as runtime, index::AttachmentIndex};
use mod_api::{prelude::*, *};
use std::{
    hint::black_box,
    sync::{Arc, Mutex},
    time::Instant,
};
#[path = "attachment_query_bench/allocations.rs"]
mod allocations;
use allocations::measured;
const HOLDERS: u32 = 10_000;
const ROUNDS: u32 = 1000;
static VALUE: AttachmentType<u32> = AttachmentType {
    namespace: "bench",
    name: "value",
    default: || panic!("no defaults"),
    codec: Some(AttachmentCodec {
        encode: |value| Ok([("n".into(), Tag::Int(*value as i32))].into()),
        decode: |saved| match saved.get(&"n".into()) {
            Some(Tag::Int(value)) => Ok(*value as u32),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};
struct BenchHost {
    index: Arc<Mutex<AttachmentIndex<AttachmentHolder>>>,
    maps: Vec<Attachments>,
}
fn holder(id: u32) -> AttachmentHolder {
    AttachmentHolder::Entity(EntityRef::new(WorldEpoch::new(1), id))
}
impl BenchHost {
    fn new(cold: bool, indexed: bool) -> Self {
        let mut host = Self {
            index: Default::default(),
            maps: Vec::with_capacity(HOLDERS as usize),
        };
        for id in 0..HOLDERS {
            let mut map = if cold {
                Attachments::from_nbt(
                    [(
                        "bench".into(),
                        Tag::Compound(
                            [(
                                "value".into(),
                                Tag::Compound(
                                    [(
                                        "value".into(),
                                        Tag::Compound([("n".into(), Tag::Int(id as i32))].into()),
                                    )]
                                    .into(),
                                ),
                            )]
                            .into(),
                        ),
                    )]
                    .into(),
                )
            } else {
                Attachments::default()
            };
            if indexed {
                map.bind_index(&host.index, holder(id));
            }
            host.maps.push(map);
            if !cold {
                VALUE.set(&mut host, Holder::Entity(id), id).unwrap();
            }
        }
        host
    }
    fn raw(&self) -> Arc<runtime::index::AttachmentSnapshot<AttachmentHolder>> {
        self.index
            .lock()
            .unwrap()
            .snapshot_named(VALUE.namespace, VALUE.name)
    }
}
impl Host for BenchHost {}
mod_api::impl_host_areas!(BenchHost, except(mod_state));
impl ModStateHost for BenchHost {
    fn attachment_query(&self, namespace: &str, name: &str) -> AttachmentBatch {
        runtime::AttachmentBatch::new(self.index.lock().unwrap().snapshot_named(namespace, name))
            .into()
    }
    fn attachments(&mut self) -> &mut dyn AttachmentStore {
        self
    }
}
impl ServerHost for BenchHost {}
mod_api::impl_server_host_areas!(BenchHost, except(mod_state));
impl ModStateServerHost for BenchHost {
    fn attachment_validate_write(
        &self,
        token: AttachmentWriteToken,
    ) -> Result<(), AttachmentRowError> {
        token.validate(&self.index.lock().unwrap())
    }
}
impl AttachmentHost for BenchHost {
    fn attachments(
        &mut self,
        holder: Holder,
        _: AttachmentAccess,
    ) -> Result<&mut Attachments, AttachmentError> {
        let Holder::Entity(id) = holder else {
            return Err(AttachmentError::MissingHolder);
        };
        self.maps
            .get_mut(id as usize)
            .ok_or(AttachmentError::MissingHolder)
    }
}
fn contribution(holder: AttachmentHolder, value: &u32) -> u64 {
    let AttachmentHolder::Entity(entity) = holder else {
        unreachable!()
    };
    u64::from(entity.id()) + u64::from(*value)
}
fn api(host: &dyn Host) -> u64 {
    host.query(&VALUE)
        .unwrap()
        .iter()
        .map(|row| {
            let row = row.unwrap();
            contribution(row.holder, row.value)
        })
        .sum()
}
fn raw(rows: &[runtime::index::AttachmentRow<AttachmentHolder>]) -> u64 {
    rows.iter()
        .map(|row| contribution(row.holder, row.entry.read(&VALUE).unwrap()))
        .sum()
}
fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}
fn timed(rounds: u32, mut f: impl FnMut() -> u64) -> f64 {
    let start = Instant::now();
    for _ in 0..rounds {
        black_box(f());
    }
    start.elapsed().as_secs_f64() * 1e6 / f64::from(rounds)
}
fn warm(host: &BenchHost) -> f64 {
    let rows = host.raw();
    let batch = host.query(&VALUE).unwrap();
    assert_eq!(api(host), 99_990_000);
    assert_eq!(raw(&rows), 99_990_000);
    let (_, counts) = measured(|| black_box(api(host)));
    assert_eq!(counts.allocations, 0);
    println!("warm acquisition + traversal allocations: {counts:?}");
    let mut baseline = Vec::new();
    let mut query = Vec::new();
    let mut raw_only = Vec::new();
    let mut typed_only = Vec::new();
    for sample in 0..10 {
        let mut pair = [0.0; 2];
        for path in if sample % 2 == 0 { [0, 1] } else { [1, 0] } {
            pair[path] = if path == 0 {
                timed(ROUNDS, || raw(&black_box(host).raw()))
            } else {
                timed(ROUNDS, || api(black_box(host as &dyn Host)))
            };
        }
        let r = timed(ROUNDS, || raw(black_box(&rows)));
        let t = timed(ROUNDS, || {
            batch
                .iter()
                .map(|row| {
                    let row = row.unwrap();
                    contribution(row.holder, row.value)
                })
                .sum()
        });
        if sample != 0 {
            baseline.push(pair[0]);
            query.push(pair[1]);
            raw_only.push(r);
            typed_only.push(t);
        }
    }
    let baseline = median(baseline);
    let query = median(query);
    let overhead = (query / baseline - 1.0) * 100.0;
    println!(
        "warm acquisition + traversal, us/10,000: raw={baseline:.3}, host.query={query:.3}, overhead={overhead:.2}%"
    );
    let raw_only = median(raw_only);
    let typed_only = median(typed_only);
    let traversal_overhead = (typed_only / raw_only - 1.0) * 100.0;
    println!(
        "warm traversal only, us/10,000: raw={raw_only:.3}, typed={typed_only:.3}, overhead={traversal_overhead:.2}%"
    );
    overhead.max(traversal_overhead)
}
fn cold() {
    for limit in [1000, HOLDERS as usize] {
        let mut lazy = Vec::new();
        let mut eager = Vec::new();
        for _ in 0..9 {
            let host = BenchHost::new(true, true);
            let start = Instant::now();
            let (sum, counts) = measured(|| {
                host.query(&VALUE)
                    .unwrap()
                    .iter()
                    .take(limit)
                    .map(|row| {
                        let row = row.unwrap();
                        contribution(row.holder, row.value)
                    })
                    .sum::<u64>()
            });
            lazy.push(start.elapsed().as_secs_f64() * 1e6);
            assert_eq!(sum, (limit * (limit - 1)) as u64);
            let host = BenchHost::new(true, true);
            let start = Instant::now();
            let (_, eager_counts) = measured(|| {
                // Eager policy measured using the very same production decode path.
                let batch = host.query(&VALUE).unwrap();
                for row in batch.iter() {
                    black_box(row.unwrap().value);
                }
                black_box(
                    batch
                        .iter()
                        .take(limit)
                        .map(|row| *row.unwrap().value as u64)
                        .sum::<u64>(),
                );
            });
            eager.push(start.elapsed().as_secs_f64() * 1e6);
            if lazy.len() == 1 {
                println!("cold {limit} rows allocations: lazy={counts:?}, eager={eager_counts:?}");
            }
        }
        println!(
            "cold {limit} rows, us: lazy={:.3}, eager={:.3}",
            median(lazy),
            median(eager)
        );
    }
}
fn cow(host: &mut BenchHost) {
    let mut pinned = Vec::new();
    let mut unpinned = Vec::new();
    for _ in 0..9 {
        let batch = host.query_mut(&VALUE).unwrap();
        let row = batch.iter().next().unwrap().unwrap();
        let start = Instant::now();
        let (_, counts) = measured(|| row.set(host, 123).unwrap());
        pinned.push(start.elapsed().as_secs_f64() * 1e6);
        if pinned.len() == 1 {
            println!("first row.set with pinned snapshot allocations: {counts:?}");
        }
        drop(batch);
        let start = Instant::now();
        let (_, ordinary_counts) = measured(|| VALUE.set(host, Holder::Entity(0), 123).unwrap());
        if unpinned.is_empty() {
            println!("unpinned ordinary set allocations: {ordinary_counts:?}");
        }
        unpinned.push(start.elapsed().as_secs_f64() * 1e6);
    }
    println!(
        "first write, us: pinned row.set={:.3}, unpinned ordinary set={:.3}",
        median(pinned),
        median(unpinned)
    );
}
static VECTOR: AttachmentType<Vec<i32>> = AttachmentType {
    namespace: "bench",
    name: "vector",
    default: || vec![0; 8],
    codec: None,
    copy_on_death: false,
    sync: false,
};
fn map_probes() {
    let mut host = BenchHost::new(false, false);
    for (label, operation) in [("get", 0), ("set", 1)] {
        let (_, counts) = measured(|| {
            for _ in 0..100_000 {
                if operation == 0 {
                    black_box(VALUE.get(&mut host, Holder::Entity(0)).unwrap());
                } else {
                    VALUE
                        .set(&mut host, Holder::Entity(0), black_box(42))
                        .unwrap();
                }
            }
        });
        println!("map {label}, 100,000 operations: {counts:?}");
        let start = Instant::now();
        for _ in 0..100_000 {
            if operation == 0 {
                black_box(VALUE.get(&mut host, Holder::Entity(0)).unwrap());
            } else {
                VALUE
                    .set(&mut host, Holder::Entity(0), black_box(42))
                    .unwrap();
            }
        }
        println!(
            "map {label}: {:.2} ns/op",
            start.elapsed().as_nanos() as f64 / 100_000.0
        );
    }
    VECTOR
        .set(&mut host, Holder::Entity(0), vec![0; 8])
        .unwrap();
    let start = Instant::now();
    let (_, counts) = measured(|| {
        for _ in 0..100_000 {
            let mut value = VECTOR.get(&mut host, Holder::Entity(0)).unwrap();
            value[0] += 1;
            VECTOR.set(&mut host, Holder::Entity(0), value).unwrap();
        }
    });
    println!(
        "map vec8 read-modify-write: {:.2} ns/op; {counts:?}",
        start.elapsed().as_nanos() as f64 / 100_000.0
    );
}
fn main() {
    map_probes();
    if std::env::var_os("ATTACHMENT_MAP_ONLY").is_some() {
        return;
    }
    if cfg!(debug_assertions) {
        eprintln!("run with --profile fast or --release");
        std::process::exit(2);
    }
    println!(
        "10,000 holders, production index/maps and dyn Host facade; profile={}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "optimized"
        }
    );
    let (unindexed, without_index) = measured(|| BenchHost::new(false, false));
    drop(unindexed);
    let (mut host, memory) = measured(|| BenchHost::new(false, true));
    println!(
        "maps + index + decoded serialized u32 entries requested live bytes: {}",
        memory.live
    );
    println!(
        "incremental residency/index requested live bytes: {}",
        memory.live - without_index.live
    );
    let overhead = warm(&host);
    cold();
    cow(&mut host);
    let before = host.raw();
    let (_, idle) = measured(|| {
        for _ in 0..10_000 {
            black_box(&host);
        }
    });
    assert_eq!(idle.allocations, 0);
    assert!(Arc::ptr_eq(&before, &host.raw()));
    println!("idle runtime maintenance control (not Game ticks): {idle:?}; bucket unchanged");
    println!("policy: retain lazy decoding; partial consumers must not decode unused rows");
    assert!(
        overhead <= 20.0,
        "warm overhead exceeds approximately 20% gate: {overhead:.2}%"
    );
}
