"""Exercise the actual animation uploader with a recording queue, without a GPU/window."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_item_action_animation import block

ROOT = Path(__file__).resolve().parents[1]


class SpriteAnimationUploadTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "rustc is required")
    def test_animation_updates_entire_layer_and_every_mip(self):
        source = (ROOT / "crates/horizon-client/src/gfx.rs").read_text()
        animate = block(source, source.index("fn animate(&mut self, tick: u64)"))
        mips = block(source, source.index("fn mip_chain("))
        program = r"""
use std::cell::RefCell;
mod wgpu {
    pub struct Texture(pub u32);
    impl Texture {pub fn width(&self)->u32 {self.0}}
    pub struct Origin3d {pub x:u32,pub y:u32,pub z:u32}
    pub enum TextureAspect {All}
    pub struct TexelCopyTextureInfo<LIFETIME> {
        pub texture:&LIFETIME Texture,pub mip_level:u32,pub origin:Origin3d,pub aspect:TextureAspect,
    }
    pub struct TexelCopyBufferLayout {pub offset:u64,pub bytes_per_row:Option<u32>,pub rows_per_image:Option<u32>}
    pub struct Extent3d {pub width:u32,pub height:u32,pub depth_or_array_layers:u32}
}
struct Queue {writes:RefCell<Vec<Vec<u8>>>}
impl Queue {
    fn write_texture(&self, info:wgpu::TexelCopyTextureInfo, pixels:&[u8], layout:wgpu::TexelCopyBufferLayout, extent:wgpu::Extent3d) {
        let size=info.texture.width() >> info.mip_level;
        assert_eq!((extent.width,extent.height,extent.depth_or_array_layers),(size,size,1),"animation must update the entire mip, not a quarter");
        assert_eq!((layout.bytes_per_row,layout.rows_per_image,layout.offset),(Some(size*4),Some(size),0));
        assert_eq!((info.origin.x,info.origin.y,info.origin.z),(0,0,7));
        assert_eq!(pixels.len(),(size*size*4) as usize);
        self.writes.borrow_mut().push(pixels.to_vec());
    }
}
struct Animation {layer:u32,pixels:Vec<u8>}
impl Animation {fn frame(&self,tick:u64)->usize {if tick==0 {0} else {1}} fn pixels(&self,_:usize)->&[u8] {&self.pixels}}
struct Gfx {animations:Vec<(Animation,usize)>,mip_levels:u32,sprite_texture:wgpu::Texture,queue:Queue}
MIPS
impl Gfx {ANIMATE}
fn main() {
    for size in [16u32,32] {
        let pixels:Vec<u8>=(0..size*size).flat_map(|i|[(i%size) as u8,(i/size) as u8,99,255]).collect();
        let (expected,levels)=mip_chain(&pixels,size,1);
        let mut gfx=Gfx {animations:vec![(Animation {layer:7,pixels:pixels.clone()},0)],mip_levels:levels,sprite_texture:wgpu::Texture(size),queue:Queue {writes:RefCell::default()}};
        gfx.animate(0);assert!(gfx.queue.writes.borrow().is_empty());
        gfx.animate(1);
        assert_eq!(gfx.queue.writes.borrow().len(),levels as usize);
        assert_eq!(gfx.queue.writes.borrow()[0],pixels,"the bottom-right texel must animate too");
        assert_eq!(gfx.queue.writes.borrow().concat(),expected);
        gfx.animate(1);assert_eq!(gfx.queue.writes.borrow().len(),levels as usize);
    }
}
""".replace("LIFETIME", chr(39)+"a").replace("MIPS", mips).replace("ANIMATE", animate)
        with tempfile.TemporaryDirectory(prefix="horizon-animation-upload-") as directory:
            path = Path(directory)
            (path / "probe.rs").write_text(program)
            subprocess.run(["rustc", "--edition=2024", "-Awarnings", str(path / "probe.rs"), "-o", str(path / "probe")], check=True, capture_output=True)
            result = subprocess.run([str(path / "probe")], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
