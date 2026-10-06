use horizon_assets::model::*;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

fn bits<const N: usize>(values: [f32; N]) -> Vec<u32> {
    values.map(f32::to_bits).to_vec()
}
fn transform(value: &DisplayTransform) -> Value {
    match value {
        DisplayTransform::Inherit => json!("inherit"),
        DisplayTransform::Explicit(None) => Value::Null,
        DisplayTransform::Explicit(Some(v)) => {
            json!({"rotation":bits(v.rotation),"translation":bits(v.translation),"scale":bits(v.scale)})
        }
    }
}
fn rotation(value: &ElementRotation) -> Value {
    let angles = match value.value {
        RotationValue::SingleAxis { axis, angle } => {
            json!({"axis": match axis { Axis::X => "x", Axis::Y => "y", Axis::Z => "z" }, "angle":angle.to_bits()})
        }
        RotationValue::EulerXyz(v) => json!(bits(v)),
    };
    json!({"origin":bits(value.origin),"value":angles,"rescale":value.rescale})
}
fn face(v: &Face) -> Value {
    json!({"cull":v.cull.map(Direction::name),"tint":v.tint_index,"texture":v.texture,"rotation":v.rotation,"uv":v.uv.map(bits)})
}
fn element(v: &Element) -> Value {
    let faces: BTreeMap<_, _> = v
        .faces
        .iter()
        .map(|(k, v)| (k.name(), v.as_ref().map(face)))
        .collect();
    json!({"from":bits(v.from),"to":bits(v.to),"faces":faces,"rotation":v.rotation.as_ref().map(rotation),"shade":v.shade_direction_override.map(Direction::name),"light":v.light_emission})
}
fn model(v: &CuboidModel) -> Value {
    let textures: BTreeMap<_, _> = v
        .textures
        .iter()
        .map(|(key, value)| {
            (
                key,
                match value {
                    TextureSlot::Reference(r) => json!({"reference":r}),
                    TextureSlot::Material(m) => {
                        json!({"sprite":m.sprite,"translucent":m.force_translucent})
                    }
                },
            )
        })
        .collect();
    let elements = v.elements.as_ref().map(|values| {
        values
            .iter()
            .map(|v| v.as_ref().map(element))
            .collect::<Vec<_>>()
    });
    let display = v.display.as_ref().map(|v| {
        DISPLAY_CONTEXTS
            .into_iter()
            .zip(v.contexts.iter().map(transform))
            .collect::<BTreeMap<_, _>>()
    });
    json!({"parent":v.parent,"ao":v.ambient_occlusion,"gui":v.gui_light.map(|v| match v { GuiLight::Front => "front", GuiLight::Side => "side" }),"textures":textures,"elements":elements,"display":display})
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut output = BufWriter::new(File::create(&args[2])?);
    for line in input.lines() {
        let row: Vec<String> = serde_json::from_str(&line?)?;
        let result = match CuboidModel::from_json(&row[1]) {
            Ok(value) => json!({"name":row[0],"model":model(&value)}),
            Err(_) => json!({"name":row[0],"error":true}),
        };
        writeln!(output, "{result}")?;
    }
    Ok(())
}
