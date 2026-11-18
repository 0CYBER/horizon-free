//! `--far D`: a bot asks the server for its far terrain (`horizon:lod`,
//! crates/horizon-client/src/lod_net.rs) as a Horizon client with LOD Distance
//! D does: the tiles of a far-terrain plan around where it stands, nearest
//! first, within the server's limits (8 waiting, 64 a second). It makes one
//! pass of the plan and says how long it took; `--far-dump DIR` keeps every
//! tile it got, to compare servers byte for byte.
use horizon_protocol::buf::{Reader, Writer};
use std::{
    collections::{HashMap, VecDeque},
    io::Write as _,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

pub const CHANNEL: &str = "horizon:lod";
/// `lod_net::IN_FLIGHT` and `PER_SECOND`.
const IN_FLIGHT: usize = 8;
const PER_SECOND: f64 = 64.0;
/// A request with no reply after this is given up.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A far tile: `x`, `z` its first block, `stride` blocks a column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tile {
    pub x: i32,
    pub z: i32,
    pub stride: i32,
}

/// The leaf tiles of `lod::Plan` (not Reduced) at `camera` for a view of
/// `detail` chunks and far terrain out to `distance` chunks, nearest first:
/// a tile splits while it is nearer than 8 of its sides, or inside the view.
pub fn plan(camera: [i32; 2], detail: i32, distance: u32) -> Vec<Tile> {
    // Roots can extend beyond the wire's i32 coordinates even at a supported distance.
    let camera = camera.map(i64::from);
    let range = i64::from(distance) * 16;
    let mut level = 0;
    while (16 << level) < range {
        level += 1;
    }
    let size = 16 << level;
    let gap = |c: i64, lo: i64, size: i64| (lo - c).max(c - lo - size).max(0);
    let mut leaves = Vec::new();
    let mut stack = Vec::new();
    for z in (camera[1] - range).div_euclid(size)..=(camera[1] + range).div_euclid(size) {
        for x in (camera[0] - range).div_euclid(size)..=(camera[0] + range).div_euclid(size) {
            stack.push((x * size, z * size, level));
        }
    }
    while let Some((x, z, level)) = stack.pop() {
        let size = 16 << level;
        let (dx, dz) = (gap(camera[0], x, size), gap(camera[1], z, size));
        let reach = dx * dx + dz * dz;
        if reach > range * range {
            continue;
        }
        let distance = (reach as f64).sqrt() as i64;
        if level > 0 && (distance < i64::from(detail) * 16 + 16 || distance < size * 8) {
            let half = size / 2;
            for (cx, cz) in [(0, 0), (half, 0), (0, half), (half, half)] {
                stack.push((x + cx, z + cz, level - 1));
            }
        } else {
            let (Ok(x), Ok(z), Ok(stride)) =
                (i32::try_from(x), i32::try_from(z), i32::try_from(size / 16))
            else {
                continue;
            };
            leaves.push((reach, Tile { x, z, stride }));
        }
    }
    leaves.sort();
    leaves.into_iter().map(|(_, tile)| tile).collect()
}

/// A request's payload (`lod_net::Request::encode`).
fn request(id: u32, tile: Tile) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(id as i32);
    w.i32(tile.x);
    w.i32(tile.z);
    w.varint(tile.stride);
    w.bool(false);
    w.data
}

#[derive(Default)]
struct State {
    /// Tiles still to ask for.
    todo: VecDeque<Tile>,
    waiting: HashMap<u32, (Tile, Instant)>,
    next: u32,
    tokens: f64,
    at: Option<Instant>,
    started: Option<Instant>,
    finished: Option<Duration>,
    served: u64,
    refused: u64,
    lost: u64,
    bytes: u64,
    planned: usize,
    latency_ms: Vec<u64>,
    dump: Option<std::io::BufWriter<std::fs::File>>,
    /// A dump couldn't be created or written: it misses tiles the counts include.
    dump_failed: bool,
}

/// One bot's far terrain.
#[derive(Default)]
pub struct Far(Mutex<State>);

impl Far {
    /// Plan the tiles around `at` (blocks), once.
    pub fn start(&self, at: [f64; 3], detail: i32, distance: u32, dump: Option<PathBuf>) {
        let mut s = self.0.lock().unwrap();
        if s.started.is_some() {
            return;
        }
        s.todo = plan(
            [at[0].floor() as i32, at[2].floor() as i32],
            detail,
            distance,
        )
        .into();
        s.planned = s.todo.len();
        s.started = Some(Instant::now());
        s.tokens = PER_SECOND;
        if let Some(path) = dump {
            match std::fs::File::create(path) {
                Ok(file) => s.dump = Some(std::io::BufWriter::new(file)),
                Err(_) => s.dump_failed = true,
            }
        }
    }

    /// Every tile of the plan asked for and answered.
    pub fn is_finished(&self) -> bool {
        self.0.lock().unwrap().finished.is_some()
    }

    /// The requests to send now (custom payload data).
    pub fn requests(&self) -> Vec<Vec<u8>> {
        let mut s = self.0.lock().unwrap();
        let now = Instant::now();
        if s.started.is_none() {
            return Vec::new();
        }
        let elapsed = s.at.map_or(0.0, |at| now.duration_since(at).as_secs_f64());
        s.tokens = (s.tokens + elapsed * PER_SECOND).min(PER_SECOND);
        s.at = Some(now);
        let late: Vec<u32> = (s.waiting.iter())
            .filter(|(_, (_, sent))| now.duration_since(*sent) > TIMEOUT)
            .map(|(id, _)| *id)
            .collect();
        for id in late {
            s.waiting.remove(&id);
            s.lost += 1;
        }
        let mut out = Vec::new();
        while s.waiting.len() < IN_FLIGHT && s.tokens >= 1.0 {
            let Some(tile) = s.todo.pop_front() else {
                break;
            };
            s.tokens -= 1.0;
            let id = s.next;
            s.next += 1;
            s.waiting.insert(id, (tile, now));
            out.push(request(id, tile));
        }
        if s.todo.is_empty() && s.waiting.is_empty() && s.finished.is_none() {
            s.finished = s.started.map(|t| t.elapsed());
        }
        out
    }

    /// A reply from the server.
    pub fn reply(&self, data: &[u8]) {
        let mut r = Reader::new(data);
        let (Ok(id), Ok(status)) = (r.varint(), r.u8()) else {
            return;
        };
        let mut s = self.0.lock().unwrap();
        let Some((tile, sent)) = s.waiting.remove(&(id as u32)) else {
            return;
        };
        s.latency_ms.push(sent.elapsed().as_millis() as u64);
        s.bytes += data.len() as u64;
        if status != 0 {
            s.refused += 1;
            return;
        }
        s.served += 1;
        let body = &data[data.len() - r.remaining()..];
        let failed = s.dump.as_mut().is_some_and(|dump| {
            let mut head = Vec::with_capacity(16);
            for v in [tile.x, tile.z, tile.stride, body.len() as i32] {
                head.extend_from_slice(&v.to_le_bytes());
            }
            dump.write_all(&head)
                .and_then(|()| dump.write_all(body))
                .is_err()
        });
        s.dump_failed |= failed;
    }

    pub fn report(&self) -> serde_json::Value {
        let mut s = self.0.lock().unwrap();
        // The dump ends here: a reply read after the report would be in the dump and not in its counts.
        if let Some(mut dump) = s.dump.take()
            && dump.flush().is_err()
        {
            s.dump_failed = true;
        }
        s.latency_ms.sort_unstable();
        let p = |q: usize| {
            s.latency_ms
                .get((s.latency_ms.len() * q / 100).min(s.latency_ms.len().saturating_sub(1)))
                .copied()
        };
        serde_json::json!({
            "planned": s.planned,
            "served": s.served,
            "refused": s.refused,
            "lost": s.lost,
            "bytes": s.bytes,
            "seconds": s.finished.map(|d| d.as_secs_f64()),
            "latency_ms_p50": p(50),
            "latency_ms_p95": p(95),
            "dump_failed": s.dump_failed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_covers_the_distance_once_coarser_farther_out() {
        let tiles = plan([8, 8], 4, 256);
        // Nearest first, finest there.
        assert_eq!(tiles[0].stride, 1);
        // No two tiles overlap.
        for (i, a) in tiles.iter().enumerate() {
            for b in &tiles[i + 1..] {
                let (sa, sb) = (a.stride * 16, b.stride * 16);
                let apart =
                    a.x + sa <= b.x || b.x + sb <= a.x || a.z + sa <= b.z || b.z + sb <= a.z;
                assert!(apart, "{a:?} {b:?}");
            }
        }
        assert!(tiles.iter().all(|t| t.stride <= 256));
    }

    #[test]
    fn the_farthest_plan_a_server_offers_is_finite() {
        let tiles = plan([0, 0], 4, 1024);
        assert!(!tiles.is_empty());
        assert!(tiles.iter().all(|t| t.stride <= 1024));
    }

    #[test]
    fn edge_cameras_keep_the_nearest_tile_and_never_wrap_requests() {
        for (camera, nearest) in [
            ([i32::MIN, i32::MAX], [i32::MIN, i32::MAX - 15]),
            ([i32::MAX, i32::MIN], [i32::MAX - 15, i32::MIN]),
            ([i32::MIN, i32::MIN], [i32::MIN, i32::MIN]),
            ([i32::MAX, i32::MAX], [i32::MAX - 15, i32::MAX - 15]),
        ] {
            let tiles = plan(camera, 4, 1024);
            assert_eq!(
                tiles.first(),
                Some(&Tile {
                    x: nearest[0],
                    z: nearest[1],
                    stride: 1,
                })
            );
            for tile in tiles {
                let size = i64::from(tile.stride) * 16;
                // The nearest point of each emitted tile stays within the requested circle.
                let [x, z] = [tile.x, tile.z].map(i64::from);
                let [cx, cz] = camera.map(i64::from);
                let dx = cx - cx.clamp(x, x + size);
                let dz = cz - cz.clamp(z, z + size);
                assert!(dx * dx + dz * dz <= 16384_i64.pow(2), "{camera:?} {tile:?}");
            }
        }
    }

    #[test]
    fn a_dump_that_cannot_be_written_is_reported_and_the_counts_stay() {
        let far = Far::default();
        // A directory can't be created as a file, on every system.
        far.start([8.0, 64.0, 8.0], 4, 8, Some(std::env::temp_dir()));
        assert!(!far.requests().is_empty());
        // Request 0 served (status 0), with a three-byte body.
        far.reply(&[0, 0, 1, 2, 3]);
        let report = far.report();
        assert_eq!(report["served"], 1);
        assert_eq!(report["dump_failed"], true);
    }
}
