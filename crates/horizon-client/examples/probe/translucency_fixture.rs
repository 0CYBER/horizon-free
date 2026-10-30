//! Transport for the vanilla quad sorting / viewpoint parity gate.
use horizon_client::gfx::translucency::{SortState, Sorter, quad_center};
use serde::Deserialize;
use serde_json::json;
use std::{
    fs,
    io::{BufWriter, Write},
};

#[derive(Deserialize)]
struct Case {
    origin: [i32; 3],
    quads: Vec<[[f32; 3]; 4]>,
    cameras: Vec<[f64; 3]>,
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        return Err("usage: translucency_fixture INPUT OUTPUT".into());
    };
    let mut out = BufWriter::new(fs::File::create(output)?);
    for line in fs::read_to_string(input)?.lines() {
        let case: Case = serde_json::from_str(line)?;
        let centers: Vec<_> = case.quads.iter().map(|q| quad_center(q[0], q[2])).collect();
        let mut state = SortState::new(centers.clone());
        let mut sorter = Sorter::default();
        let mut cache_sorter = Sorter::default();
        let frames: Vec<_> = case
            .cameras
            .iter()
            .map(|&camera| {
                let update = state
                    .update(camera, case.origin, &mut cache_sorter)
                    .is_some();
                let local = std::array::from_fn(|i| (camera[i] - f64::from(case.origin[i])) as f32);
                json!({"indices": sorter.sort(&centers, local), "update": update})
            })
            .collect();
        let bits: Vec<_> = centers
            .iter()
            .map(|c| c.map(|f| f.to_bits() as i32))
            .collect();
        writeln!(out, "{}", json!({"centers": bits, "frames": frames}))?;
    }
    Ok(())
}
