use horizon_assets::animation::{AnimationMetadata, AnimationSample};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};
fn probe(row: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let m = AnimationMetadata::from_json(row["json"].as_str().unwrap())?;
    let width = row["width"].as_i64().unwrap() as i32;
    let height = row["height"].as_i64().unwrap() as i32;
    let size = m.frame_size(width, height);
    let frames = m.frames.as_ref().map(|frames| {
        frames
            .iter()
            .map(|f| json!({"index":f.index,"time":f.time}))
            .collect::<Vec<_>>()
    });
    let mut result = json!({"metadata":{"frames":frames,"width":m.width,"height":m.height,"frametime":m.frame_time,"interpolate":m.interpolate},"size":size});
    if size[0] > width || size[1] > height || width % size[0] != 0 || height % size[1] != 0 {
        return Ok(result);
    }
    result["animation"] = match m.clip(((width / size[0]) * (height / size[1])) as u32)? {
        None => Value::Null,
        Some(clip) => {
            let samples: Vec<_> = (0..=row["ticks"].as_u64().unwrap())
                .map(|tick| sample(clip.sample(tick)))
                .collect();
            let mut edges = Vec::new();
            let mut start = 0u64;
            for &(_, duration) in clip.sequence() {
                let duration = i64::from(duration);
                for sub in [0, 1, duration / 2, duration - 2, duration - 1] {
                    if sub >= 0 && sub < duration {
                        edges.push(sample(clip.sample(start + sub as u64 + 1)));
                    }
                }
                start += duration as u64;
            }
            json!({"sequence":clip.sequence(),"columns":width/size[0],"samples":samples,"edges":edges})
        }
    };
    Ok(result)
}
fn sample(v: AnimationSample) -> Value {
    json!([
        v.sequence_index,
        v.sub_frame,
        v.frame,
        v.next_frame,
        v.dirty,
        v.needs_draw,
        v.interpolation_millis
    ])
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut output = BufWriter::new(File::create(&args[2])?);
    for line in input.lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let result = match probe(&row) {
            Ok(value) => json!({"name":row["name"],"value":value}),
            Err(_) => json!({"name":row["name"],"error":true}),
        };
        writeln!(output, "{result}")?;
    }
    Ok(())
}
