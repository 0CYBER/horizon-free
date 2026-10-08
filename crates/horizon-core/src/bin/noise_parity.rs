//! Test transport for comparing noise with the downloaded vanilla artifacts.
use horizon_core::{
    geometry::Axis,
    noise::{
        DensityVolume, GradientTable, Noise, NoiseBuildError, NoiseStack, NormalNoise,
        Normalization, PerlinNoise, SimplexNoise, SmearedPerlinNoise, wrap,
    },
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use std::io::{self, BufRead, BufWriter, Write};
use std::sync::Arc;

enum Sampler {
    Perlin(PerlinNoise),
    Simplex(SimplexNoise),
    Smeared(SmearedPerlinNoise),
    Stack(NoiseStack),
}
impl Sampler {
    fn table(&self) -> &GradientTable {
        match self {
            Self::Perlin(n) => n.table(),
            Self::Simplex(n) => n.table(),
            Self::Smeared(n) => n.table(),
            Self::Stack(_) => panic!("stack has no gradient table"),
        }
    }
    fn as_noise(&self) -> &dyn Noise {
        match self {
            Self::Perlin(n) => n,
            Self::Simplex(n) => n,
            Self::Smeared(n) => n,
            Self::Stack(n) => n,
        }
    }
    fn shared_noise(&self) -> Arc<dyn Noise> {
        match self {
            Self::Perlin(n) => Arc::new(n.clone()),
            Self::Simplex(n) => Arc::new(n.clone()),
            Self::Smeared(n) => Arc::new(n.clone()),
            Self::Stack(n) => Arc::new(n.clone()),
        }
    }
    fn range(&self) -> Result<[f32; 2], horizon_core::noise::NanRange> {
        self.as_noise().range()
    }
    fn get2(&self, x: f64, z: f64) -> f32 {
        self.as_noise().get2(x, z)
    }
    fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        self.as_noise().get3(x, y, z)
    }
}
fn read<T: std::str::FromStr>(
    args: &mut std::str::SplitWhitespace<'_>,
) -> Result<T, Box<dyn std::error::Error>> {
    args.next()
        .ok_or("missing argument")?
        .parse()
        .map_err(|_| "invalid argument".into())
}
fn double(args: &mut std::str::SplitWhitespace<'_>) -> Result<f64, Box<dyn std::error::Error>> {
    Ok(f64::from_bits(u64::from_str_radix(
        args.next().ok_or("missing double")?,
        16,
    )?))
}
fn float(args: &mut std::str::SplitWhitespace<'_>) -> Result<f32, Box<dyn std::error::Error>> {
    Ok(f32::from_bits(u32::from_str_radix(
        args.next().ok_or("missing float")?,
        16,
    )?))
}
fn bits(v: f32) -> u32 {
    if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() }
}
fn dbits(v: f64) -> u64 {
    if v.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        v.to_bits()
    }
}
fn create<R: RandomSource>(
    kind: &str,
    mut rng: R,
    skip: bool,
    scale: f64,
    out: &mut impl Write,
) -> io::Result<Sampler> {
    let sampler = match kind {
        "perlin" => Sampler::Perlin(PerlinNoise::new(&mut rng)),
        "simplex" => Sampler::Simplex(SimplexNoise::with_skipped_offsets(&mut rng, skip)),
        "smeared" => Sampler::Smeared(SmearedPerlinNoise::new(&mut rng, scale)),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown sampler",
            ));
        }
    };
    let offsets = sampler.table().offsets();
    let range = sampler.range();
    let (lower, upper) = match range {
        Ok(range) => (bits(range[0]).to_string(), bits(range[1]).to_string()),
        Err(_) => ("error".to_owned(), "error".to_owned()),
    };
    writeln!(
        out,
        "create {} {} {} {} {} {}",
        dbits(offsets[0]),
        dbits(offsets[1]),
        dbits(offsets[2]),
        lower,
        upper,
        rng.next_long()
    )?;
    write!(out, "permutations")?;
    for v in sampler.table().permutations() {
        write!(out, " {v}")?;
    }
    writeln!(out)?;
    Ok(sampler)
}
fn normal_probe<R: RandomSource>(
    out: &mut impl Write,
    definition: Result<NormalNoise, NoiseBuildError>,
    mut random: R,
    points: &[[f64; 3]],
    legacy: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let Ok(definition) = definition else {
        writeln!(out, "normal error {}", random.next_long())?;
        return Ok(());
    };
    let range = definition.range();
    writeln!(
        out,
        "normal {} {} {} {}",
        definition.octaves().len(),
        dbits(definition.normalization_factor()),
        bits(range[0]),
        bits(range[1])
    )?;
    for octave in definition.octaves() {
        writeln!(
            out,
            "octave {} {} {} {}",
            octave.octave_index,
            dbits(octave.frequency),
            dbits(octave.amplitude),
            octave.seed()
        )?;
    }
    let created = if legacy {
        definition.create_for_legacy_nether_biome(&mut random)
    } else {
        definition.create(&mut random)
    };
    match created {
        Err(_) => writeln!(out, "normal-create error")?,
        Ok(noise) => {
            let range = noise.range()?;
            writeln!(
                out,
                "normal-create {} {} {}",
                noise.layers().len(),
                bits(range[0]),
                bits(range[1])
            )?;
            for layer in noise.layers() {
                writeln!(
                    out,
                    "normal-layer {} {} {} {}",
                    dbits(layer.frequency),
                    bits(layer.amplitude),
                    bits(layer.noise.get2(0.25, 0.75)),
                    bits(layer.noise.get3(-17.25, 64.125, 32.5))
                )?;
            }
            for &[x, y, z] in points {
                writeln!(
                    out,
                    "normal-query {} {}",
                    bits(noise.get2(x, z)),
                    bits(noise.get3(x, y, z))
                )?;
            }
            let volume = DensityVolume::new([2, 5, 2], [-17, -64, 32], [4, 1, 4])?;
            let mut values: Vec<f32> = (0..volume.size())
                .map(|i| (i as f32 - 10.0) * 0.125)
                .collect();
            for (pass, amplitude) in [0.5, -0.25].into_iter().enumerate() {
                noise.add_to_volume(&mut values, volume, 0.125, 0.25, amplitude)?;
                for (i, value) in values.iter().enumerate() {
                    writeln!(out, "normal-volume {pass} {i} {}", bits(*value))?;
                }
            }
        }
    }
    writeln!(out, "normal-parent {}", random.next_long())?;
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let mut out = BufWriter::new(io::stdout().lock());
    let mut samplers = Vec::new();
    for line in stdin.lock().lines() {
        let line = line?;
        let mut args = line.split_whitespace();
        match args.next().ok_or("missing operation")? {
            "normal" => {
                let kind = args.next().ok_or("missing RNG kind")?;
                let seed: i64 = read(&mut args)?;
                let parity: i32 = read(&mut args)?;
                let base: i32 = read(&mut args)?;
                let count: i32 = read(&mut args)?;
                let normalization: i32 = read(&mut args)?;
                let amplitude = double(&mut args)?;
                let modifiers: usize = read(&mut args)?;
                let modifiers = (0..modifiers)
                    .map(|_| double(&mut args))
                    .collect::<Result<Vec<_>, _>>()?;
                let point_count: usize = read(&mut args)?;
                let points = (0..point_count)
                    .map(|_| Ok([double(&mut args)?, double(&mut args)?, double(&mut args)?]))
                    .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
                let definition = if parity & 1 != 0 {
                    NormalNoise::create_parity(base, &modifiers)
                } else {
                    (|| {
                        let mut builder = NormalNoise::builder();
                        builder
                            .set_base_octave(base)
                            .set_base_amplitude(amplitude)
                            .set_normalization(match normalization {
                                0 => Normalization::Disabled,
                                1 => Normalization::Enabled,
                                _ => Normalization::Legacy,
                            });
                        builder.set_octave_count(count)?;
                        for (index, value) in modifiers.iter().enumerate() {
                            builder.set_amplitude_modifier(index as i32, *value)?;
                        }
                        builder.build()
                    })()
                };
                match kind {
                    "legacy" => normal_probe(
                        &mut out,
                        definition,
                        LegacyRandom::new(seed),
                        &points,
                        parity & 2 != 0,
                    )?,
                    "xoroshiro" => normal_probe(
                        &mut out,
                        definition,
                        XoroshiroRandom::new(seed),
                        &points,
                        parity & 2 != 0,
                    )?,
                    _ => return Err("unknown stream".into()),
                }
            }
            "create" => {
                let kind = args.next().ok_or("missing sampler")?;
                let stream = args.next().ok_or("missing stream")?;
                let seed: i64 = read(&mut args)?;
                let skip: i32 = read(&mut args)?;
                let scale = double(&mut args)?;
                let sampler = match stream {
                    "legacy" => create(kind, LegacyRandom::new(seed), skip != 0, scale, &mut out)?,
                    "xoroshiro" => {
                        create(kind, XoroshiroRandom::new(seed), skip != 0, scale, &mut out)?
                    }
                    _ => return Err("unknown stream".into()),
                };
                samplers.push(sampler);
            }
            operation @ ("query" | "local") => {
                let id: usize = read(&mut args)?;
                let mut point = [double(&mut args)?, double(&mut args)?, double(&mut args)?];
                let mut derivative = [float(&mut args)?, float(&mut args)?, float(&mut args)?];
                let sampler = &samplers[id];
                if operation == "local" {
                    for (value, offset) in point.iter_mut().zip(sampler.table().offsets()) {
                        *value -= offset;
                    }
                }
                let [x, y, z] = point;
                write!(
                    out,
                    "query {} {}",
                    bits(sampler.get2(x, z)),
                    bits(sampler.get3(x, y, z))
                )?;
                let value = match sampler {
                    Sampler::Perlin(n) => Some(n.noise_with_derivative(x, y, z, &mut derivative)),
                    Sampler::Smeared(n) => Some(n.noise_with_derivative(x, y, z, &mut derivative)),
                    Sampler::Simplex(_) | Sampler::Stack(_) => None,
                };
                if let Some(value) = value {
                    write!(
                        out,
                        " {} {} {} {}",
                        bits(value),
                        bits(derivative[0]),
                        bits(derivative[1]),
                        bits(derivative[2])
                    )?;
                }
                writeln!(out)?;
            }
            "volume" => {
                let id: usize = read(&mut args)?;
                let sizes = [read(&mut args)?, read(&mut args)?, read(&mut args)?];
                let minimum = [read(&mut args)?, read(&mut args)?, read(&mut args)?];
                let steps = [read(&mut args)?, read(&mut args)?, read(&mut args)?];
                let xz_scale = double(&mut args)?;
                let y_scale = double(&mut args)?;
                let amplitude = float(&mut args)?;
                let seed: i64 = read(&mut args)?;
                let repeats: usize = read(&mut args)?;
                match DensityVolume::new(sizes, minimum, steps) {
                    Err(_) => writeln!(out, "volume error")?,
                    Ok(volume) => {
                        let mut random = LegacyRandom::new(seed);
                        let mut values: Vec<f32> = (0..volume.size() + 4)
                            .map(|_| f32::from_bits(random.next_int() as u32))
                            .collect();
                        let sampler = &samplers[id];
                        for _ in 0..repeats {
                            sampler.as_noise().add_to_volume(
                                &mut values,
                                volume,
                                xz_scale,
                                y_scale,
                                amplitude,
                            )?;
                        }
                        let maximum = volume.maximum();
                        writeln!(
                            out,
                            "volume {} {} {} {}",
                            volume.size(),
                            maximum[0],
                            maximum[1],
                            maximum[2]
                        )?;
                        let [sx, sy, sz] = volume.sizes();
                        for z in 0..sz {
                            for x in 0..sx {
                                for y in 0..sy {
                                    let index = volume.index([x, y, z]).unwrap();
                                    writeln!(
                                        out,
                                        "cell {index} {} {} {} {}",
                                        volume.block(Axis::X, x).unwrap(),
                                        volume.block(Axis::Y, y).unwrap(),
                                        volume.block(Axis::Z, z).unwrap(),
                                        bits(values[index])
                                    )?;
                                }
                            }
                        }
                        for value in &values[volume.size()..] {
                            writeln!(out, "tail {}", bits(*value))?;
                        }
                    }
                }
            }
            "stack" => {
                let count: usize = read(&mut args)?;
                let mut builder = NoiseStack::builder();
                for _ in 0..count {
                    let flatten: i32 = read(&mut args)?;
                    let id: usize = read(&mut args)?;
                    let frequency = double(&mut args)?;
                    let amplitude = float(&mut args)?;
                    if flatten != 0 {
                        let Sampler::Stack(stack) = &samplers[id] else {
                            return Err("flatten expects a stack".into());
                        };
                        builder.add_stack(stack, frequency, amplitude);
                    } else {
                        builder.add(samplers[id].shared_noise(), frequency, amplitude);
                    }
                }
                match builder.build() {
                    Err(_) => writeln!(out, "stack error")?,
                    Ok(stack) => {
                        let range = stack.range()?;
                        writeln!(
                            out,
                            "stack {} {} {}",
                            stack.layers().len(),
                            bits(range[0]),
                            bits(range[1])
                        )?;
                        for (index, layer) in stack.layers().iter().enumerate() {
                            writeln!(
                                out,
                                "layer {} {} {}",
                                dbits(layer.frequency),
                                bits(layer.amplitude),
                                bits(stack.get_layer(index).unwrap().get3(0.0, 0.0, 0.0))
                            )?;
                        }
                        samplers.push(Sampler::Stack(stack));
                    }
                }
            }
            "wrap" => writeln!(out, "wrap {}", dbits(wrap(double(&mut args)?)))?,
            _ => return Err("unknown operation".into()),
        }
        if args.next().is_some() {
            return Err("extra argument".into());
        }
    }
    out.flush()?;
    Ok(())
}
