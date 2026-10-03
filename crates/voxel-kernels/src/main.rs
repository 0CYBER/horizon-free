use std::{hint::black_box, time::Instant};
use voxel_kernels::Section;

fn sample(corpus: &[Section], kernel: fn(&Section) -> u32, repeats: usize) -> f64 {
    let start = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..repeats {
        for section in corpus {
            checksum = checksum.wrapping_add(u64::from(kernel(black_box(section))));
        }
    }
    black_box(checksum);
    start.elapsed().as_nanos() as f64 / (corpus.len() * repeats) as f64
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn main() {
    let repeats: usize = match std::env::args().nth(1) {
        None => 100,
        Some(value) => match value.parse() {
            Ok(n) if n > 0 => n,
            _ => {
                eprintln!("usage: voxel-bench [positive repeat count]");
                std::process::exit(2);
            }
        },
    };
    let mut rng = 0x7065_7273_6f6e_616c_u64;
    let random = |rng: &mut u64| {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 7;
        *rng ^= *rng << 17;
        *rng
    };
    println!(
        "workload,sections,samples,repeats,scalar_ns_per_section,bitset_ns_per_section,speedup"
    );
    for workload in [
        "empty",
        "solid",
        "flat",
        "checkerboard",
        "random_10",
        "random_50",
        "random_90",
    ] {
        let corpus: Vec<_> = (0..32)
            .map(|i| {
                Section::from_fn(|x, y, z| match workload {
                    "empty" => false,
                    "solid" => true,
                    "flat" => y < 1 + i % 15,
                    "checkerboard" => (x + y + z + i) % 2 == 0,
                    "random_10" => random(&mut rng) % 100 < 10,
                    "random_50" => random(&mut rng) % 100 < 50,
                    "random_90" => random(&mut rng) % 100 < 90,
                    _ => unreachable!(),
                })
            })
            .collect();
        for section in &corpus {
            assert_eq!(
                section.exposed_faces_scalar(),
                section.exposed_faces_bitset()
            );
        }
        for kernel in [Section::exposed_faces_scalar, Section::exposed_faces_bitset] {
            sample(&corpus, kernel, 10);
        }
        let mut scalar = Vec::new();
        let mut bitset = Vec::new();
        for trial in 0..21 {
            if trial % 2 == 0 {
                scalar.push(sample(&corpus, Section::exposed_faces_scalar, repeats));
                bitset.push(sample(&corpus, Section::exposed_faces_bitset, repeats));
            } else {
                bitset.push(sample(&corpus, Section::exposed_faces_bitset, repeats));
                scalar.push(sample(&corpus, Section::exposed_faces_scalar, repeats));
            }
        }
        let scalar = median(&mut scalar);
        let bitset = median(&mut bitset);
        println!(
            "{workload},32,21,{repeats},{scalar:.2},{bitset:.2},{:.2}",
            scalar / bitset
        );
    }
}
