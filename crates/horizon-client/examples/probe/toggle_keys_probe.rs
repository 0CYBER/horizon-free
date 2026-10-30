#![forbid(unsafe_code)]
//! Runs tools/test_toggle_keys_parity.py's event sequences through
//! `ToggleKeys`, the sneak/sprint/attack/use toggle mappings.
use horizon_client::{
    Result,
    keymapping::{KeyAction, ToggleKeys},
};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

pub fn main() -> Result<()> {
    let args: Vec<String> = crate::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut out = BufWriter::new(File::create(&args[2])?);
    for (n, line) in input.lines().enumerate() {
        let c: Value = serde_json::from_str(&line?)?;
        writeln!(out, "== {n}")?;
        let mut keys = ToggleKeys::default();
        let mut options = [false; 4];
        for op in c["ops"].as_array().unwrap() {
            let name = op[0].as_str().unwrap();
            let slot = || op[1].as_u64().unwrap() as usize;
            match name {
                "option" => options[slot()] = op[2].as_bool().unwrap(),
                "key" => {
                    let action = match op[2].as_i64().unwrap() {
                        0 => KeyAction::Release,
                        1 => KeyAction::Press,
                        _ => KeyAction::Repeat,
                    };
                    keys.key(slot(), action, op[3].as_bool().unwrap(), options[slot()]);
                }
                "button" => keys.button(
                    slot(),
                    op[2].as_bool().unwrap(),
                    op[3].as_bool().unwrap(),
                    options[slot()],
                ),
                "open" => keys.screen_opened(options),
                "close" => keys.screen_closed(options),
                "respawn" => keys.respawn(),
                other => panic!("{other}"),
            }
            let mut line = name.to_string();
            for k in &keys.0 {
                line += &format!(" {} {}", k.is_down, k.pending_clicks());
            }
            writeln!(out, "{line}")?;
        }
    }
    out.flush()?;
    Ok(())
}
