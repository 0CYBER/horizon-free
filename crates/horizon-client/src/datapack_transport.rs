//! Bounded custom-payload fragments; activate only after the complete snapshot validates.
use super::*;

const MAGIC: &[u8; 4] = b"DRS1";
const HEADER: usize = 12;
pub const CHUNK_DATA: usize = 512 << 10;
static ASSEMBLY: Mutex<Assembly> = Mutex::new(Assembly { pending: None });

pub fn chunks(bytes: &[u8]) -> Result<impl Iterator<Item = Vec<u8>> + '_, String> {
    if bytes.is_empty() || bytes.len() > mod_api::MAX_SNAPSHOT_BYTES {
        return Err("Datapack registry snapshot exceeds the 64 MiB transport limit.".into());
    }
    let total = bytes.len() as u32;
    Ok(bytes.chunks(CHUNK_DATA).enumerate().map(move |(i, part)| {
        let mut packet = Vec::with_capacity(HEADER + part.len());
        packet.extend_from_slice(MAGIC);
        packet.extend_from_slice(&total.to_le_bytes());
        packet.extend_from_slice(&((i * CHUNK_DATA) as u32).to_le_bytes());
        packet.extend_from_slice(part);
        packet
    }))
}

#[derive(Default)]
struct Assembly {
    pending: Option<(usize, Vec<u8>)>,
}
impl Assembly {
    fn push(&mut self, packet: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let previous = self.pending.take();
        let bad = || "Invalid or out-of-order datapack registry snapshot fragment.".to_string();
        if packet.len() < HEADER || &packet[..4] != MAGIC {
            return Err(bad());
        }
        let total = u32::from_le_bytes(packet[4..8].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(packet[8..12].try_into().unwrap()) as usize;
        if total == 0 || total > mod_api::MAX_SNAPSHOT_BYTES {
            return Err("Datapack registry snapshot exceeds the 64 MiB transport limit.".into());
        }
        if offset >= total || packet.len() - HEADER != CHUNK_DATA.min(total - offset) {
            return Err(bad());
        }
        let mut bytes = if offset == 0 {
            if previous.is_some() {
                return Err(bad());
            }
            Vec::with_capacity(total)
        } else {
            let (expected, bytes) = previous.ok_or_else(bad)?;
            if total != expected || bytes.len() != offset {
                return Err(bad());
            }
            bytes
        };
        bytes.extend_from_slice(&packet[HEADER..]);
        if bytes.len() == total {
            Ok(Some(bytes))
        } else {
            self.pending = Some((total, bytes));
            Ok(None)
        }
    }
}

pub fn clear_transfer() {
    ASSEMBLY.lock().unwrap_or_else(|e| e.into_inner()).pending = None;
}
pub fn finish_transfer() -> Result<(), String> {
    if ASSEMBLY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pending
        .is_some()
    {
        Err("Incomplete datapack registry snapshot at the end of configuration.".into())
    } else {
        Ok(())
    }
}
pub fn accept_chunk(packet: &[u8]) -> Result<(), String> {
    let complete = ASSEMBLY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(packet)?;
    if let Some(bytes) = complete {
        accept(&bytes)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "datapack_transport_tests.rs"]
mod tests;
