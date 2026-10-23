//! WeatherEffectRenderer column instances and quads, without a GPU.
use horizon_client::weather;
use std::{fmt::Write, fs};
fn bits(value: f32) -> String {
    if value.is_nan() {
        "nan".into()
    } else {
        format!("{:x}", value.to_bits())
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let mut output = String::new();
    let mut parameters = weather::ParameterCache::default();
    for (case, line) in fs::read_to_string(&args[1])?.lines().enumerate() {
        let p: Vec<&str> = line.split_whitespace().collect();
        let int = |i: usize| p[i].parse::<i32>().unwrap();
        let kind = int(0);
        let ticks: i64 = p[1].parse()?;
        let (x, bottom, top, z, light) = (int(2), int(3), int(4), int(5), int(6));
        let partial: f32 = p[7].parse()?;
        let camera = [p[8].parse()?, p[9].parse()?, p[10].parse()?];
        let radius = int(11);
        let intensity: f32 = p[12].parse()?;
        let mut random = weather::column_random(x, z);
        let column = if kind == 0 {
            weather::rain_column(&mut random, ticks, x, bottom, top, z, light, partial)
        } else {
            weather::snow_column(&mut random, ticks, x, bottom, top, z, light, partial)
        };
        write!(
            output,
            "{case} {} {} {} {} {} {} {}",
            column.x,
            column.z,
            column.bottom_y,
            column.top_y,
            bits(column.u_offset),
            bits(column.v_offset),
            column.light_coords
        )?;
        assert_eq!(
            parameters
                .get(x, z)
                .column(kind != 0, ticks, x, bottom, top, z, light, partial),
            column,
            "cached parameters differ from original random source at case {case}"
        );
        let max_alpha = if kind == 0 { 1.0 } else { 0.8 };
        for v in weather::column_quad(&column, camera, max_alpha, radius, intensity) {
            write!(
                output,
                " |{},{},{},{},{},{:x},{}",
                bits(v.pos[0]),
                bits(v.pos[1]),
                bits(v.pos[2]),
                bits(v.uv[0]),
                bits(v.uv[1]),
                v.color,
                v.light_coords
            )?;
        }
        output.push('\n');
    }
    fs::write(&args[2], output)?;
    Ok(())
}
