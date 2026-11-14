#[cfg(test)]
mod benches {
    use crate::save::{
        Record,
        compression_pool::{CompressionPool, compress_records_with_pool},
    };
    use horizon_world::nbt::{NamedTag, Tag};
    use std::{sync::Arc, time::Instant};

    fn sample_chunk_tag(x: i32, z: i32) -> NamedTag {
        let payload: Vec<i64> = (0..2000)
            .map(|v| (v as i64) ^ ((x * 31 + z) as i64))
            .collect();
        NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("xPos", Tag::Int(x)),
                ("zPos", Tag::Int(z)),
                ("DataVersion", Tag::Int(3953)),
                ("Status", Tag::String("full".into())),
                ("Sections", Tag::LongArray(payload)),
            ]),
        }
    }

    #[test]
    #[ignore = "a measurement, not a check"]
    fn bench_chunk_compression_speedup() {
        eprintln!("\n=== CHUNK COMPRESSION WORKER POOL BENCHMARK ===");
        eprintln!("CPU: 13th Gen Intel Core i5-1334U (12 threads)");
        eprintln!("----------------------------------------------------------------------");
        eprintln!("Chunk Count | Sequential (1T) | Async Pool (6 Workers) | Speedup");
        eprintln!("------------+-----------------+------------------------+----------");

        let pool_multi = CompressionPool::new(6);
        let pool_single = CompressionPool::new(0);

        for &count in &[1, 2, 3, 4, 8, 16, 32, 64] {
            let records: Vec<((i32, i32), Record)> = (0..count)
                .map(|i| ((i, i), Record::Tag(Arc::new(sample_chunk_tag(i, -i)))))
                .collect();
            let refs: Vec<_> = records.iter().map(|(k, r)| (*k, r)).collect();

            // Warmup
            let _ = compress_records_with_pool(&refs, 100, &pool_multi);

            let iters = if count <= 8 { 100 } else { 25 };

            let t0 = Instant::now();
            for _ in 0..iters {
                let _ = compress_records_with_pool(&refs, 100, &pool_single);
            }
            let seq_time = t0.elapsed() / iters;

            let t1 = Instant::now();
            for _ in 0..iters {
                let _ = compress_records_with_pool(&refs, 100, &pool_multi);
            }
            let pool_time = t1.elapsed() / iters;

            let speedup = seq_time.as_secs_f64() / pool_time.as_secs_f64();

            eprintln!(
                "{:>11} | {:>15.3?} | {:>22.3?} | {:>7.2}x",
                count, seq_time, pool_time, speedup
            );
        }
        eprintln!("----------------------------------------------------------------------\n");
    }
}
