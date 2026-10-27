"""Cushion particles must reach clients even when the server has no sprite atlas."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(shutil.which("rustc"), "requires Rust")
class CushionParticlesTests(unittest.TestCase):
    def test_headless_server_records_break_effect_before_sprite_lookup(self):
        source = (ROOT / "crates/horizon-client/src/game/cushions.rs").read_text()
        start = source.index("    fn cushion_break_particles(")
        end = source.index("    /// `BlockAttachedEntity.tick`", start)
        fixture = r'''
#![allow(dead_code)]
#[derive(Clone, Copy)]
struct Vec3 { x: f64, y: f64, z: f64 }
struct Cushion { position: Vec3 }
impl Cushion { fn color_name(&self) -> &str { "blue" } }
struct Registry;
impl Registry {
    fn default_state(&self, name: &str) -> Option<u32> {
        assert_eq!(name, "minecraft:blue_wool"); Some(123)
    }
}
struct Assets { registry: Registry }
struct Pack { assets: Assets }
struct Audio { capture: Option<()> }
mod horizon_client { pub mod particles {
    pub enum ParticleType { Block }
    pub struct ParticleOptions;
    impl ParticleOptions {
        pub fn block(_: ParticleType, _: String, _: u32) -> Self { Self }
    }
} }
#[derive(Default)]
struct Particles { count: u32 }
impl Particles {
    fn send_particles(&mut self, _: &horizon_client::particles::ParticleOptions,
        at: [f64; 3], spread: [f64; 3], speed: f64, count: u32) {
        assert!((at[1] - (10.0 + 1.0/6.0)).abs() < 1e-12);
        assert_eq!(spread, [0.25, 0.0625, 0.25]);
        assert_eq!(speed, 0.05); assert_eq!(count, 10); self.count += count;
    }
}
struct Game {
    pack: Pack, audio: Audio, particles: Particles, headless: bool,
    lan_cushion_breaks: Vec<(u32, [f64; 3])>,
}
impl Game {
    fn particle_sprite(&self, _: u32, _: [i32; 3]) -> Option<(String, u32)> {
        (!self.headless).then(|| ("block/blue_wool".into(), 0xffffff))
    }
METHOD
}
fn main() {
    let cushion = Cushion { position: Vec3 { x: 1.5, y: 10.0, z: 2.5 } };
    for headless in [true, false] {
        for serving in [true, false] {
            let mut game = Game {
                pack: Pack { assets: Assets { registry: Registry } },
                audio: Audio { capture: serving.then_some(()) },
                particles: Particles::default(), headless, lan_cushion_breaks: vec![],
            };
            game.cushion_break_particles(&cushion);
            assert_eq!(game.lan_cushion_breaks.len(), usize::from(serving));
            if serving {
                let (state, at) = game.lan_cushion_breaks[0];
                assert_eq!(state, 123);
                assert_eq!([at[0], at[2]], [1.5, 2.5]);
                assert!((at[1] - (10.0 + 1.0/6.0)).abs() < 1e-12);
            }
            assert_eq!(game.particles.count, if headless { 0 } else { 10 });
        }
    }
}
'''.replace("METHOD", source[start:end])
        with tempfile.TemporaryDirectory(prefix="cushion-particles-") as directory:
            path = Path(directory)
            (path / "main.rs").write_text(fixture)
            subprocess.run(["rustc", "--edition=2024", str(path / "main.rs"), "-o", str(path / "probe")],
                           check=True, capture_output=True, text=True)
            result = subprocess.run([str(path / "probe")], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
