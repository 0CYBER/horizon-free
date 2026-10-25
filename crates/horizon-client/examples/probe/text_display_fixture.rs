use horizon_client::text_display::TextDisplayData;
use horizon_protocol::{ids, packets::play::Clientbound};
use std::{error::Error, io::Write};

pub fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = crate::args().collect();
    let mut out = std::fs::File::create(&args[2])?;
    for line in std::fs::read_to_string(&args[1])?.lines() {
        let input: serde_json::Value = serde_json::from_str(line)?;
        let hex = input["wire"].as_str().unwrap();
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<Result<_, _>>()?;
        let Clientbound::SetEntityData(packet) =
            Clientbound::decode(ids::play::clientbound::SET_ENTITY_DATA, &bytes)?
        else {
            unreachable!()
        };
        let mut display = TextDisplayData::default();
        display.apply(&packet.values);
        if let Some(angles) = input["angles"].as_array() {
            let a: Vec<_> = angles.iter().map(|v| v.as_f64().unwrap() as f32).collect();
            writeln!(
                out,
                "{}",
                serde_json::to_string(&display.matrix([a[0], a[1]], [a[2], a[3]]).to_cols_array())?
            )?;
            continue;
        }
        let mut state = serde_json::to_value(&display)?;
        state["text"] = display.text.plain().into();
        state["alignment"] = format!("{:?}", display.align()).to_uppercase().into();
        writeln!(out, "{}", state)?;
    }
    Ok(())
}
