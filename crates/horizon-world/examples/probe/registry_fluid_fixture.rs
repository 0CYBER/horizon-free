use horizon_core::{
    geometry::{Direction, VoxelShape},
    position::BlockPos,
};
use horizon_world::{
    block_support::BlockSupportRegistry,
    dynamic_support::DynamicSupportWorld,
    fluid_flow,
    nbt::{self, Limits},
    registry::{BlockRegistry, BlockStateId},
    registry_fluid::{FluidBlockWorld, FluidRegistry, RegistryFluidWorld},
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Write},
    path::Path,
};
fn pos(p: BlockPos) -> String {
    format!("{} {} {}", p.x, p.y, p.z)
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
}
impl World<'_> {
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected registered fluid query failure"))
        } else {
            Ok(())
        }
    }
    fn cell(&self, p: BlockPos) -> BlockStateId {
        BlockStateId(
            self.f["cells"]
                .get(pos(p))
                .unwrap_or(&self.f["default"])
                .as_u64()
                .unwrap() as u32,
        )
    }
}
impl FluidBlockWorld for World<'_> {
    fn block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        self.query(format!("block {}", pos(p)))?;
        Ok(self.cell(p))
    }
    fn fluid_block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        self.query(format!("fluid {}", pos(p)))?;
        Ok(self.cell(p))
    }
}
impl DynamicSupportWorld for World<'_> {
    type Error = io::Error;
    fn shulker_is_closed(&mut self, p: [i32; 3]) -> io::Result<Option<bool>> {
        self.query(format!("entity {} {} {}", p[0], p[1], p[2]))?;
        Ok(None)
    }
    fn moving_piston_shape(&mut self, p: [i32; 3]) -> io::Result<Option<VoxelShape>> {
        self.query(format!("entity {} {} {}", p[0], p[1], p[2]))?;
        Ok(None)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    let load = |n: &str| -> Result<_, Box<dyn std::error::Error>> {
        Ok(nbt::read_named(&fs::read(dir.join(n))?, Limits::default())?
            .0
            .tag)
    };
    let registry = BlockRegistry::from_nbt(&load("block-registry.nbt")?)?;
    let support = BlockSupportRegistry::from_nbt(&load("cached-support.nbt")?, &registry)?;
    let info: Value = serde_json::from_slice(&fs::read(dir.join("fluid-info.json"))?)?;
    let names = |v: &Value| -> BTreeSet<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    };
    let natural = names(&info["blocks_flow"]);
    let ice = names(&info["ice"]);
    let all = (0..registry.len())
        .map(|i| registry.state(BlockStateId(i as u32)).unwrap().name.clone())
        .collect();
    let bindings = [
        FluidRegistry::new(&registry, &natural, &ice)?,
        FluidRegistry::new(&registry, &BTreeSet::new(), &ice)?,
        FluidRegistry::new(&registry, &all, &ice)?,
    ];
    assert!(bindings[0].fluid(BlockStateId(u32::MAX)).is_err());
    assert!(
        FluidRegistry::new(
            &registry,
            &BTreeSet::from(["horizon:absent".to_owned()]),
            &ice
        )
        .is_err()
    );
    let foreign = BlockRegistry::from_nbt(&load("block-registry.nbt")?)?;
    let foreign_binding = FluidRegistry::new(&foreign, &natural, &ice)?;
    let dummy = serde_json::json!({"fail":-1,"cells":{},"default":u32::MAX});
    let mut sink = io::sink();
    let mut probe = World {
        f: &dummy,
        out: &mut sink,
        calls: 0,
    };
    assert!(RegistryFluidWorld::new(&foreign_binding, &support, &mut probe).is_err());
    let mut bound = RegistryFluidWorld::new(&bindings[0], &support, &mut probe)?;
    assert!(matches!(
        horizon_world::fluid_flow::FluidFlowWorld::fluid(&mut bound, BlockPos::new(0, 0, 0)),
        Err(horizon_world::registry_fluid::FluidQueryError::Data(_))
    ));
    assert!(matches!(
        horizon_world::fluid_flow::FluidFlowWorld::block(&mut bound, BlockPos::new(0, 0, 0)),
        Err(horizon_world::registry_fluid::FluidQueryError::Data(_))
    ));
    assert_eq!(probe.calls, 2);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let bindings = &bindings[f["tag"].as_u64().unwrap() as usize];
        let mut world = World {
            f: &f,
            out: &mut out,
            calls: 0,
        };
        let a = &f["position"];
        let p = BlockPos::new(
            a[0].as_i64().unwrap() as i32,
            a[1].as_i64().unwrap() as i32,
            a[2].as_i64().unwrap() as i32,
        );
        let root = bindings.fluid(BlockStateId(f["root"].as_u64().unwrap() as u32))?;
        let mut w = RegistryFluidWorld::new(bindings, &support, &mut world)?;
        let result = match f["mode"].as_str().unwrap() {
            "flow" => fluid_flow::flow_at(&mut w, root, p).map(|v| {
                format!(
                    "flow {:016x} {:016x} {:016x}",
                    v.x.to_bits(),
                    v.y.to_bits(),
                    v.z.to_bits()
                )
            }),
            "face" => fluid_flow::solid_face(
                &mut w,
                root.kind(),
                p,
                Direction::ALL[f["direction"].as_u64().unwrap() as usize],
            )
            .map(|v| format!("face {v}")),
            "height" => root
                .height_at(&mut w, [p.x, p.y, p.z])
                .map(|v| format!("height {:08x}", v.to_bits())),
            "camera" => root
                .camera_height_at(&mut w, [p.x, p.y, p.z])
                .map(|v| format!("height {:08x}", v.to_bits())),
            _ => panic!("fixture mode"),
        };
        writeln!(world.out, "{}", result.unwrap_or_else(|_| "error".into()))?;
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
