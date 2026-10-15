//! What goes over the wire: an install's items as one JSON object, zstd-compressed
//! (`Content-Encoding: zstd`). The server decodes with [`decode`], which bounds what it inflates.
use std::io::Read;

use serde::{Deserialize, Serialize};

use crate::Item;
use crate::consent::is_install_id;

/// A batch on the wire, compressed.
pub const MAX_COMPRESSED: usize = 256 * 1024;
/// A batch's JSON: more than the largest item, so any item fits on its own.
pub const MAX_JSON: usize = 1024 * 1024;
pub const MAX_ITEMS: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Batch {
    pub schema: u32,
    pub install_id: String,
    pub items: Vec<Item>,
}

impl Batch {
    pub fn new(install_id: &str, items: Vec<Item>) -> Self {
        Self {
            schema: crate::SCHEMA,
            install_id: install_id.to_owned(),
            items,
        }
    }

    pub fn json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

pub fn encode(batch: &Batch) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(
        batch.json().as_slice(),
        ruzstd::encoding::CompressionLevel::Fastest,
    )
}

/// Inflates and checks a batch: its size before and after, its schema and install ID, and
/// every item.
pub fn decode(bytes: &[u8]) -> Result<Batch, String> {
    if bytes.len() > MAX_COMPRESSED {
        return Err(format!("over {MAX_COMPRESSED} bytes"));
    }
    let decoder = ruzstd::decoding::StreamingDecoder::new(bytes).map_err(|e| e.to_string())?;
    let mut json = Vec::new();
    decoder
        .take(MAX_JSON as u64 + 1)
        .read_to_end(&mut json)
        .map_err(|e| e.to_string())?;
    if json.len() > MAX_JSON {
        return Err(format!("over {MAX_JSON} bytes inflated"));
    }
    let batch: Batch = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    if batch.schema == 0 || batch.schema > crate::SCHEMA {
        return Err(format!("unknown schema {}", batch.schema));
    }
    if !is_install_id(&batch.install_id) {
        return Err("not an install ID".into());
    }
    if batch.items.is_empty() || batch.items.len() > MAX_ITEMS {
        return Err(format!("1 to {MAX_ITEMS} items"));
    }
    batch.items.iter().try_for_each(Item::validate)?;
    Ok(batch)
}

/// Splits items into batches of at most [`MAX_ITEMS`] and [`MAX_JSON`] bytes, in order.
pub fn split<T>(items: Vec<(T, Item)>) -> Vec<Vec<(T, Item)>> {
    let mut batches: Vec<Vec<(T, Item)>> = Vec::new();
    let mut size = 0;
    for (tag, item) in items {
        let len = serde_json::to_vec(&item).map_or(0, |j| j.len()) + 1;
        let full = batches
            .last()
            .is_none_or(|b| b.len() >= MAX_ITEMS || size + len > MAX_JSON - 1024);
        if full {
            batches.push(Vec::new());
            size = 0;
        }
        size += len;
        if let Some(batch) = batches.last_mut() {
            batch.push((tag, item));
        }
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Crash, Payload, Perf, Source, Timings};

    const ID: &str = "0123456789abcdef0123456789abcdef";

    fn perf() -> Item {
        let mut t = Timings::new();
        (0..50_000).for_each(|i| t.record(6.0 + (i % 97) as f32 / 10.0));
        Item::new(
            Source::Game,
            "v0.4.2",
            Payload::Perf(Perf {
                frames: t.summary(),
                seconds: 600,
                ..Perf::default()
            }),
        )
    }

    #[test]
    fn a_batch_round_trips_and_compresses() {
        let batch = Batch::new(ID, vec![perf(); 12]);
        let bytes = encode(&batch);
        assert_eq!(decode(&bytes).unwrap(), batch);
        let json = batch.json().len();
        assert!(bytes.len() * 4 < json, "{} of {json} bytes", bytes.len());
        assert!(
            bytes.len() < 2048,
            "a day's sessions: {} bytes",
            bytes.len()
        );
    }

    #[test]
    fn bad_batches_are_refused() {
        assert!(decode(b"not zstd").is_err());
        assert!(
            decode(&vec![0; MAX_COMPRESSED + 1])
                .unwrap_err()
                .contains("over")
        );
        let mut batch = Batch::new("someone", vec![perf()]);
        assert!(decode(&encode(&batch)).unwrap_err().contains("install ID"));
        batch.install_id = ID.into();
        batch.schema = crate::SCHEMA + 1;
        assert!(decode(&encode(&batch)).unwrap_err().contains("schema"));
        batch.schema = crate::SCHEMA;
        batch.items.clear();
        assert!(decode(&encode(&batch)).is_err());
        let huge = Item::new(
            Source::Game,
            "v1",
            Payload::Crash(Crash {
                stack: "x".repeat(40_000),
                ..Crash::default()
            }),
        );
        assert!(
            decode(&encode(&Batch::new(ID, vec![huge])))
                .unwrap_err()
                .contains("over its")
        );
    }

    #[test]
    fn a_zip_bomb_stops_at_the_limit() {
        // Compresses to a few kilobytes, inflates past the limit.
        let json = format!(
            r#"{{"schema":1,"install_id":"{ID}","items":[],"pad":"{}"}}"#,
            " ".repeat(MAX_JSON * 4)
        );
        let bytes = ruzstd::encoding::compress_to_vec(
            json.as_bytes(),
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        assert!(bytes.len() < MAX_COMPRESSED);
        assert!(decode(&bytes).unwrap_err().contains("inflated"));
    }

    #[test]
    fn items_split_into_batches_by_count_and_size() {
        let items: Vec<(usize, Item)> = (0..150).map(|i| (i, perf())).collect();
        let batches = split(items);
        assert_eq!(
            batches.iter().map(Vec::len).collect::<Vec<_>>(),
            [64, 64, 22]
        );
        assert_eq!(batches[2].last().unwrap().0, 149);
        let big = || {
            Item::new(
                Source::Launcher,
                "0.1.5",
                Payload::Benchmark(serde_json::json!({"pad": "x".repeat(120_000)})),
            )
        };
        let batches = split((0..20).map(|i| (i, big())).collect());
        assert!(batches.len() >= 3);
        for batch in batches {
            let json = Batch::new(ID, batch.into_iter().map(|b| b.1).collect()).json();
            assert!(json.len() <= MAX_JSON);
        }
    }
}
