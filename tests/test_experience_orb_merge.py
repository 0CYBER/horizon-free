"""Run the real orb merge code (spawn and tick) with a fake mod."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from rust_items import rust_fn

FIXTURE = r"""
#[derive(Clone, Copy)] struct Vec3 { x: f64, y: f64, z: f64 }
impl Vec3 {
    fn new(x: f64, y: f64, z: f64) -> Self { Vec3 { x, y, z } }
    fn subtract(self, o: Vec3) -> Vec3 { Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z) }
    fn add_vector(self, o: Vec3) -> Vec3 { Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z) }
}
mod join_seq { pub type JoinSeq = u32; }
mod orb_limits { pub const MAX_COUNT: u32 = i32::MAX as u32; }
mod mod_api { pub mod __private {
    pub struct ExperienceOrbMergeCall<'a> {
        pub dimension: &'a str, pub into: u32, pub from: u32, pub into_value: i32,
        pub into_count: u32, pub from_value: i32, pub from_count: u32, pub canceled: &'a mut bool,
    }
} }
#[derive(Clone, Copy)] struct Aabb { lo: Vec3, hi: Vec3 }
impl Aabb {
    fn new(lo: Vec3, hi: Vec3) -> Self { Aabb { lo, hi } }
    fn inflate(self, d: Vec3) -> Aabb { Aabb::new(self.lo.subtract(d), self.hi.add_vector(d)) }
    fn intersects(self, o: Aabb) -> bool {
        self.lo.x < o.hi.x && self.hi.x > o.lo.x && self.lo.y < o.hi.y && self.hi.y > o.lo.y
            && self.lo.z < o.hi.z && self.hi.z > o.lo.z
    }
}
struct ExperienceOrb { join: u32, id: u32, x: f64, value: i32, count: u32, age: u32, ticks: u32 }
impl ExperienceOrb {
    fn bounds(&self) -> Aabb {
        Aabb::new(Vec3::new(self.x - 0.25, 0.0, -0.25), Vec3::new(self.x + 0.25, 0.5, 0.25))
    }
    fn can_merge(&self, id: u32, value: i32) -> bool {
        self.value == value && (self.id as i32).wrapping_sub(id as i32) % 40 == 0
    }
    fn points(&self) -> u32 { self.value.max(0) as u32 }
}
trait Locate { fn locate(&self, join: u32) -> Option<usize>; }
impl Locate for Vec<ExperienceOrb> {
    fn locate(&self, join: u32) -> Option<usize> { self.iter().position(|o| o.join == join) }
}
struct Dim;
impl Dim { fn id(&self) -> &'static str { "minecraft:overworld" } }
struct Game { orbs: Vec<ExperienceOrb>, remote: Option<()>, dim: Dim, added: Vec<u32>,
              flip: bool, cancel: bool, asked: Vec<(u32, u32, i32, i32)>, merged: Vec<(u32, u32)>,
              // What a mod's merge hook writes with set_experience_orb: into's value and count,
              // and a change to the source's value that makes the decision stale.
              write: Option<(i32, u32)>, stale: bool, seen: Vec<(i32, u32, i32, u32)> }
impl Game {
    fn native_experience_orb_can_merge(&mut self, _: &str, into: u32, from: u32, iv: i32, fv: i32,
                                       can_merge: &mut bool) {
        self.asked.push((into, from, iv, fv));
        if self.flip { *can_merge = !*can_merge; }
    }
    fn native_experience_orb_merge(&mut self, call: mod_api::__private::ExperienceOrbMergeCall<'_>) {
        let _ = call.dimension;
        self.merged.push((call.into, call.from));
        self.seen.push((call.into_value, call.into_count, call.from_value, call.from_count));
        *call.canceled = self.cancel;
        if let Some((value, count)) = self.write {
            let o = self.orbs.iter_mut().find(|o| o.id == call.into).unwrap();
            (o.value, o.count) = (value, count);
        }
        if self.stale {
            self.orbs.iter_mut().find(|o| o.id == call.from).unwrap().value += 1;
        }
    }
    fn add_orb(&mut self, _: [f64; 3], value: u32) { self.added.push(value); }
    RESTORE_ORB
    ORB_MERGES
    AWARD_ORB
    MERGE_NEW_ORB
    MERGE_ORB
}
fn orb(id: u32, x: f64, value: i32, ticks: u32) -> ExperienceOrb {
    ExperienceOrb { join: id, id, x, value, count: 1, age: 10, ticks }
}
fn game(orbs: Vec<ExperienceOrb>, flip: bool, cancel: bool) -> Game {
    Game { orbs, remote: None, dim: Dim, added: vec![], flip, cancel, asked: vec![], merged: vec![],
           write: None, stale: false, seen: vec![] }
}
fn counted(id: u32, x: f64, value: i32, count: u32) -> ExperienceOrb {
    ExperienceOrb { count, ..orb(id, x, value, 0) }
}
#[test]
fn a_merge_hook_write_is_the_merged_orb() {
    // 3 points x2 takes 5 points x1: the mod writes the exact total, 11 x1.
    let mut g = game(vec![counted(1, 0.0, 3, 2), counted(41, 0.2, 5, 1)], true, false);
    g.write = Some((11, 1));
    g.merge_orb(0);
    assert_eq!(g.seen, [(3, 2, 5, 1)]);
    assert_eq!((g.orbs.len(), g.orbs[0].value, g.orbs[0].count), (1, 11, 1));
    // A spawning 5 into 3 x2: from is 0 and counts as 1; the write is not counted again.
    let mut g = game(vec![counted(41, 0.0, 3, 2)], true, false);
    g.write = Some((11, 1));
    g.award_orb([0.0, 0.0, 0.0], 5, 1);
    assert_eq!((g.merged.clone(), g.seen.clone()), (vec![(41, 0)], vec![(3, 2, 5, 1)]));
    assert_eq!((g.orbs[0].value, g.orbs[0].count, g.orbs[0].age, g.added.len()), (11, 1, 0, 0));
    // A canceled merge keeps both orbs as they were, whatever the mod wrote to into.
    let mut g = game(vec![counted(1, 0.0, 3, 2), counted(41, 0.2, 5, 1)], true, true);
    g.write = Some((11, 1));
    g.merge_orb(0);
    let left: Vec<_> = g.orbs.iter().map(|o| (o.value, o.count)).collect();
    assert_eq!(left, [(3, 2), (5, 1)]);
    // The same for a spawning orb: the write is put back and a fresh orb spawns.
    let mut g = game(vec![counted(41, 0.0, 3, 2)], true, true);
    g.write = Some((11, 1));
    g.award_orb([0.0, 0.0, 0.0], 5, 1);
    assert_eq!((g.orbs[0].value, g.orbs[0].count, g.orbs[0].age), (3, 2, 10));
    assert_eq!((g.orbs.len(), g.added.clone()), (1, vec![5]));
}
#[test]
fn a_spawning_merge_tells_the_hook_the_signed_value() {
    // A destination of -7 x2 is reported as it is, not clamped to the 0 points it pays.
    let mut g = game(vec![counted(41, 0.0, -7, 2)], true, false);
    g.award_orb([0.0, 0.0, 0.0], 5, 1);
    assert_eq!(g.seen, [(-7, 2, 5, 1)]);
}
#[test]
fn a_stale_or_overflowing_merge_leaves_both_orbs() {
    // The source changed during the hook: the write to into is put back.
    let mut g = game(vec![counted(1, 0.0, 3, 2), counted(41, 0.2, 5, 1)], true, false);
    g.write = Some((11, 1));
    g.stale = true;
    g.merge_orb(0);
    let left: Vec<_> = g.orbs.iter().map(|o| (o.value, o.count)).collect();
    assert_eq!(left, [(3, 2), (6, 1)]);
    // A count past what a save holds refuses rather than saturating.
    let mut g = game(vec![counted(1, 0.0, 3, i32::MAX as u32), counted(41, 0.2, 3, 1)], false, false);
    g.merge_orb(0);
    let left: Vec<_> = g.orbs.iter().map(|o| o.count).collect();
    assert_eq!(left, [i32::MAX as u32, 1]);
    let mut g = game(vec![counted(41, 0.0, 3, i32::MAX as u32)], false, false);
    g.award_orb([0.0, 0.0, 0.0], 3, 1);
    assert_eq!((g.orbs[0].count, g.added.clone()), (i32::MAX as u32, vec![3]));
}
#[test]
fn spawn_merge_is_vanilla_unless_a_mod_decides() {
    // vanilla: same value, ids 40 apart -> merges
    let mut g = game(vec![orb(41, 0.0, 3, 0)], false, false);
    g.award_orb([0.0, 0.0, 0.0], 3, 1);
    assert_eq!((g.orbs[0].count, g.orbs[0].age, g.added.len()), (2, 0, 0));
    assert_eq!(g.asked, [(41, 0, 3, 3)]);
    // different value: vanilla refuses, a mod flips it
    let mut g = game(vec![orb(41, 0.0, 3, 0)], true, false);
    g.award_orb([0.0, 0.0, 0.0], 7, 1);
    assert_eq!(g.orbs[0].count, 2);
    // vanilla yes, a mod flips to no -> fresh orb
    let mut g = game(vec![orb(41, 0.0, 3, 0)], true, false);
    g.award_orb([0.0, 0.0, 0.0], 3, 1);
    assert_eq!((g.orbs[0].count, g.added.clone()), (1, vec![3]));
    // canceled -> fresh orb; out of reach is not asked
    let mut g = game(vec![orb(41, 0.0, 3, 0), orb(81, 5.0, 3, 0)], false, true);
    g.award_orb([0.0, 0.0, 0.0], 3, 1);
    assert_eq!((g.orbs[0].count, g.added.clone(), g.merged.clone()), (1, vec![3], vec![(41, 0)]));
    // a client of a server never merges or asks
    let mut g = game(vec![orb(41, 0.0, 3, 0)], false, false);
    g.remote = Some(());
    g.award_orb([0.0, 0.0, 0.0], 3, 1);
    assert_eq!((g.orbs[0].count, g.added.clone(), g.asked.len()), (1, vec![3], 0));
}
#[test]
fn tick_merge_is_vanilla_unless_a_mod_decides() {
    let two = |a, b| vec![orb(1, 0.0, 3, a), orb(41, 0.2, 3, b)];
    let mut g = game(two(0, 0), false, false);
    g.merge_orb(0);
    assert_eq!((g.orbs.len(), g.orbs[0].count, g.orbs[0].age), (1, 2, 10));
    assert_eq!(g.asked, [(1, 41, 3, 3)]);
    // Hooks see the signed value, including values which yield no pickup points.
    for value in [-7, 0] {
        let mut g = game(vec![orb(1, 0.0, value, 0), orb(41, 0.2, value, 0)], false, false);
        g.merge_orb(0);
        assert_eq!(g.asked, [(1, 41, value, value)]);
        assert_eq!((g.orbs.len(), g.orbs[0].count), (1, 2));
    }
    // only an orb on its scan tick asks: tickCount 20 is not the cadence
    let mut g = game(two(19, 0), false, false);
    g.merge_orb(0);
    assert_eq!((g.orbs.len(), g.asked.len()), (2, 0));
    // vanilla yes, mod no
    let mut g = game(two(0, 0), true, false);
    g.merge_orb(0);
    assert_eq!(g.orbs.len(), 2);
    // different ids: vanilla no, mod yes
    let mut g = game(vec![orb(1, 0.0, 3, 0), orb(2, 0.2, 3, 5)], true, false);
    g.merge_orb(0);
    assert_eq!((g.orbs.len(), g.orbs[0].count), (1, 2));
    // canceled merge keeps both
    let mut g = game(two(0, 0), false, true);
    g.merge_orb(0);
    assert_eq!((g.orbs.len(), g.merged.clone()), (2, vec![(1, 41)]));
    // every eligible orb in reach is absorbed in the one scan
    let mut g = game(vec![orb(1, 0.0, 3, 0), orb(41, 0.2, 3, 0), orb(81, 0.4, 3, 5)], false, false);
    g.merge_orb(0);
    assert_eq!((g.orbs.len(), g.orbs[0].count), (1, 3));
}
"""


class ExperienceOrbMergeTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_spawn_and_tick_merge_hooks(self):
        fixture = (FIXTURE.replace("RESTORE_ORB", rust_fn("restore_orb", owner="Game"))
                   .replace("ORB_MERGES", rust_fn("orb_merges"))
                   .replace("AWARD_ORB", rust_fn("award_orb"))
                   .replace("MERGE_NEW_ORB", rust_fn("merge_new_orb"))
                   .replace("MERGE_ORB", rust_fn("merge_orb", owner="Game")))
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "orb.rs", Path(folder) / "orb.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
