use horizon_protocol::{
    buf::{Codec, Reader, Writer},
    packets::play::StopSound,
};
use std::{error::Error, io::Write};
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut out = std::fs::File::create(&args[2])?;
    for line in std::fs::read_to_string(&args[1])?.lines() {
        let input: serde_json::Value = serde_json::from_str(line)?;
        let hex = input["wire"].as_str().unwrap();
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<Result<_, _>>()?;
        let mut reader = Reader::new(&bytes);
        let packet = StopSound::read(&mut reader)?;
        assert_eq!(reader.remaining(), 0);
        let mut writer = Writer::new();
        StopSound::write(&packet, &mut writer);
        assert_eq!(writer.data, bytes);
        writeln!(
            out,
            "{}",
            serde_json::json!({"source":packet.source,"sound":packet.sound})
        )?;
    }
    assert!(StopSound::read(&mut Reader::new(&[1, 11])).is_err());
    assert!(StopSound::read(&mut Reader::new(&[3])).is_err());
    Ok(())
}
