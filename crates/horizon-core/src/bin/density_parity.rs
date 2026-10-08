//! Original line transport for differential tests of density evaluation.
use horizon_core::density::graph::{
    DensityCompiler, DensityGraph, FunctionId, GraphBinary, GraphNode, GraphUnary, NoiseId,
};
use horizon_core::{
    density::{
        BinaryOp, BinarySampler, BlendDensitySampler, BlendedNoise, CachingDensitySampler,
        ConstantSampler, ContextBoundSampler, CubicSpline, DensitySampler, DensityValue,
        DistanceMetric, DistanceToPointSampler, EmptyBlender, FindTopSurfaceSampler,
        GradientSampler, InterpolatedSampler, Interval, IntervalError, IntervalSelectSampler,
        LerpSampler, NoiseSampler, PowerSampler, RangeChoiceSampler, RoundMode, RoundSampler,
        SamplerContext, ShiftNoiseKind, ShiftedNoiseSampler, SliceAxes, SliceSampler, SplinePoint,
        SplineSampler, TilingMode, UnaryOp, UnarySampler, WorldField, WorldFields,
        compile_shift_noise, compile_surface_search, create_blended_fbm,
    },
    geometry::Axis,
    noise::{
        DensityVolume, Noise, NormalNoise, Normalization, PerlinNoise, SimplexNoise,
        SmearedPerlinNoise,
    },
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use std::{
    io::{self, BufRead, BufWriter, Write},
    sync::Arc,
};
#[derive(Debug)]
struct CountedSampler {
    input: Arc<dyn DensitySampler>,
    calls: Arc<[std::sync::atomic::AtomicUsize; 2]>,
}
impl DensitySampler for CountedSampler {
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, horizon_core::density::SampleError> {
        self.calls[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.input.sample_value(context, position)
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), horizon_core::density::SampleError> {
        self.calls[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.input.sample_volume(context, buffer, volume)
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
fn float(args: &mut std::str::SplitWhitespace<'_>) -> Result<f32, Box<dyn std::error::Error>> {
    Ok(f32::from_bits(u32::from_str_radix(
        args.next().ok_or("missing float")?,
        16,
    )?))
}
fn double(args: &mut std::str::SplitWhitespace<'_>) -> Result<f64, Box<dyn std::error::Error>> {
    Ok(f64::from_bits(u64::from_str_radix(
        args.next().ok_or("missing double")?,
        16,
    )?))
}
fn bits(v: f32) -> u32 {
    if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() }
}
fn noise(kind: &str, mut random: impl RandomSource, scale: f64) -> Arc<dyn Noise> {
    match kind {
        "normal" => {
            let mut definition = NormalNoise::builder();
            definition.set_base_octave(-7);
            definition.set_octave_count(4).unwrap();
            Arc::new(definition.build().unwrap().create(&mut random).unwrap())
        }
        "perlin" => Arc::new(PerlinNoise::new(&mut random)),
        "simplex" => Arc::new(SimplexNoise::new(&mut random)),
        "smeared" => Arc::new(SmearedPerlinNoise::new(&mut random, scale)),
        _ => panic!("unknown noise kind"),
    }
}
fn interval_output(
    out: &mut impl Write,
    name: &str,
    result: Result<Interval, IntervalError>,
) -> io::Result<()> {
    match result {
        Ok(range) => writeln!(
            out,
            "bounds {name} {} {} {} {}",
            bits(range.min()),
            bits(range.max()),
            u8::from(range.is_nai()),
            range.java_hash_code()
        ),
        Err(_) => writeln!(out, "bounds {name} error"),
    }
}
fn blended_probe(
    random: &mut impl RandomSource,
    definition: BlendedNoise,
    out: &mut impl Write,
) -> Result<Option<Arc<dyn DensitySampler>>, Box<dyn std::error::Error>> {
    interval_output(out, "blended", definition.range())?;
    let result = definition.compile(random);
    writeln!(
        out,
        "blended {} {}",
        if result.is_ok() { "ok" } else { "error" },
        random.next_long()
    )?;
    Ok(result
        .ok()
        .map(|value| Arc::new(value) as Arc<dyn DensitySampler>))
}
fn fbm_probe(
    random: &mut impl RandomSource,
    octave: i32,
    smear: f64,
    amplitude: f64,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = create_blended_fbm(random, octave, smear, amplitude);
    match result {
        Ok(stack) => {
            let range = stack.range()?;
            writeln!(
                out,
                "fbm {} {} {}",
                stack.layers().len(),
                bits(range[0]),
                bits(range[1])
            )?;
            for layer in stack.layers() {
                writeln!(
                    out,
                    "fbm-layer {} {} {}",
                    layer.frequency.to_bits(),
                    bits(layer.amplitude),
                    bits(layer.noise.get3(-17.25, 64.125, 32.5))
                )?;
            }
            for point in [
                [0.0; 3],
                [-17.25, 64.125, 32.5],
                [3e7, -64.0, -3e7],
                [f64::INFINITY; 3],
            ] {
                writeln!(
                    out,
                    "fbm-query {} {}",
                    bits(stack.get2(point[0], point[2])),
                    bits(stack.get3(point[0], point[1], point[2]))
                )?;
            }
            let volume = DensityVolume::new([2, 5, 2], [-17, -64, 32], [4, 1, 4])?;
            let mut buffer = vec![0.0; volume.size() + 2];
            for (i, v) in buffer.iter_mut().enumerate() {
                *v = (i as f32 - 10.0) * 0.125;
            }
            stack.add_to_volume(&mut buffer, volume, 0.125, 0.25, -0.5)?;
            for value in buffer {
                writeln!(out, "fbm-cell {}", bits(value))?;
            }
        }
        Err(_) => writeln!(out, "fbm error")?,
    }
    writeln!(out, "fbm-parent {}", random.next_long())?;
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut samplers: Vec<Arc<dyn DensitySampler>> = Vec::new();
    let mut graph_compiler = DensityCompiler::new(DensityGraph::new());
    let mut functions: Vec<FunctionId> = Vec::new();
    let mut graph_splines: Vec<CubicSpline<FunctionId>> = Vec::new();
    let mut graph_noises: Vec<NoiseId> = Vec::new();
    let mut noises = Vec::new();
    let mut splines: Vec<CubicSpline> = Vec::new();
    let mut counters: Vec<Arc<[std::sync::atomic::AtomicUsize; 2]>> = Vec::new();
    let mut context = SamplerContext::default();
    let mut out = BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line?;
        let mut args = line.split_whitespace();
        match args.next().ok_or("empty command")? {
            "constant" => {
                samplers.push(Arc::new(ConstantSampler(float(&mut args)?)));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "gradient" => {
                let axis = Axis::ALL[read::<usize>(&mut args)?];
                let tiling = [
                    TilingMode::ClampToEdge,
                    TilingMode::Repeat,
                    TilingMode::MirroredRepeat,
                ][read::<usize>(&mut args)?];
                let from = read(&mut args)?;
                let to = read(&mut args)?;
                let low = float(&mut args)?;
                let high = float(&mut args)?;
                samplers.push(Arc::new(GradientSampler::new(
                    axis, tiling, from, to, low, high,
                )));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "noise" => {
                let kind = args.next().ok_or("missing noise kind")?;
                let stream = args.next().ok_or("missing stream")?;
                let seed = read(&mut args)?;
                let scale = double(&mut args)?;
                let source = if stream == "legacy" {
                    noise(kind, LegacyRandom::new(seed), scale)
                } else {
                    noise(kind, XoroshiroRandom::new(seed), scale)
                };
                let sampler = NoiseSampler {
                    noise: source,
                    xz_scale: double(&mut args)?,
                    y_scale: double(&mut args)?,
                };
                noises.push(sampler.clone());
                samplers.push(Arc::new(sampler));
                writeln!(out, "noise {} {}", noises.len() - 1, samplers.len() - 1)?;
            }
            "shift" => {
                let source = noises[read::<usize>(&mut args)?].clone();
                let x = read::<usize>(&mut args)?;
                let y = read::<i32>(&mut args)?;
                let z = read::<usize>(&mut args)?;
                samplers.push(Arc::new(ShiftedNoiseSampler {
                    source,
                    shift_x: samplers[x].clone(),
                    shift_y: if y < 0 {
                        None
                    } else {
                        Some(samplers[y as usize].clone())
                    },
                    shift_z: samplers[z].clone(),
                }));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "unary" => {
                let kind = args.next().ok_or("missing unary kind")?;
                let input = samplers[read::<usize>(&mut args)?].clone();
                let a = float(&mut args)?;
                let b = float(&mut args)?;
                let operation = match kind {
                    "abs" => UnaryOp::Abs,
                    "negate" => UnaryOp::Negate,
                    "square" => UnaryOp::Square,
                    "cube" => UnaryOp::Cube,
                    "sqrt" => UnaryOp::Sqrt,
                    "log" => UnaryOp::Log,
                    "reciprocal" => UnaryOp::Reciprocal,
                    "sign" => UnaryOp::Sign,
                    "squeeze" => UnaryOp::Squeeze,
                    "leaky" => UnaryOp::LeakyRelu(a),
                    "add" => UnaryOp::Add(a),
                    "sub" => UnaryOp::SubtractFrom(a),
                    "mul" => UnaryOp::Multiply(a),
                    "div" => UnaryOp::DivideInto(a),
                    "min" => UnaryOp::Minimum(a),
                    "max" => UnaryOp::Maximum(a),
                    "clamp" => UnaryOp::Clamp { min: a, max: b },
                    _ => return Err("unknown unary kind".into()),
                };
                samplers.push(Arc::new(UnarySampler { input, operation }));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "power" => {
                let mode = read::<u8>(&mut args)?;
                let sampler: Arc<dyn DensitySampler> = match mode {
                    0 => Arc::new(PowerSampler {
                        base: samplers[read::<usize>(&mut args)?].clone(),
                        exponent: samplers[read::<usize>(&mut args)?].clone(),
                    }),
                    1 => {
                        let base = double(&mut args)?;
                        Arc::new(UnarySampler {
                            input: samplers[read::<usize>(&mut args)?].clone(),
                            operation: UnaryOp::PowerBase(base),
                        })
                    }
                    2 => {
                        let base = samplers[read::<usize>(&mut args)?].clone();
                        Arc::new(UnarySampler {
                            input: base,
                            operation: UnaryOp::PowerExponent(double(&mut args)?),
                        })
                    }
                    _ => return Err("unknown power mode".into()),
                };
                samplers.push(sampler);
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "round" => {
                let mode = [
                    RoundMode::Floor,
                    RoundMode::Round,
                    RoundMode::Ceil,
                    RoundMode::Truncate,
                ][read::<usize>(&mut args)?];
                let input = samplers[read::<usize>(&mut args)?].clone();
                let multiple = read::<i32>(&mut args)?;
                let multiple = if multiple < 0 {
                    None
                } else {
                    Some(samplers[multiple as usize].clone())
                };
                samplers.push(Arc::new(RoundSampler {
                    mode,
                    input,
                    multiple,
                }));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "binary" => {
                let kind = args.next().ok_or("missing binary kind")?;
                let left = samplers[read::<usize>(&mut args)?].clone();
                let right = samplers[read::<usize>(&mut args)?].clone();
                let bound = float(&mut args)?;
                let operation = match kind {
                    "add" => BinaryOp::Add,
                    "sub" => BinaryOp::Subtract,
                    "mul" => BinaryOp::Multiply,
                    "div" => BinaryOp::Divide,
                    "min" => BinaryOp::Minimum { right_min: bound },
                    "max" => BinaryOp::Maximum { right_max: bound },
                    _ => return Err("unknown binary kind".into()),
                };
                samplers.push(Arc::new(BinarySampler {
                    left,
                    right,
                    operation,
                }));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "choice" => {
                let mode = read::<u8>(&mut args)?;
                let input = samplers[read::<usize>(&mut args)?].clone();
                let min_inclusive = float(&mut args)?;
                let max_exclusive = float(&mut args)?;
                let sampler: Arc<dyn DensitySampler> = if mode == 1 {
                    let inside = float(&mut args)?;
                    let outside = float(&mut args)?;
                    Arc::new(UnarySampler {
                        input,
                        operation: UnaryOp::ConstantRangeChoice {
                            min_inclusive,
                            max_exclusive,
                            inside,
                            outside,
                        },
                    })
                } else {
                    let when_in_range = samplers[read::<usize>(&mut args)?].clone();
                    let when_out_of_range = samplers[read::<usize>(&mut args)?].clone();
                    Arc::new(RangeChoiceSampler {
                        input,
                        min_inclusive,
                        max_exclusive,
                        when_in_range,
                        when_out_of_range,
                    })
                };
                samplers.push(sampler);
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "lerp" => {
                let mode = read::<u8>(&mut args)?;
                let alpha = samplers[read::<usize>(&mut args)?].clone();
                let first = if mode == 1 {
                    DensityValue::Constant(float(&mut args)?)
                } else {
                    DensityValue::Sampler(samplers[read::<usize>(&mut args)?].clone())
                };
                let second = if mode == 2 {
                    DensityValue::Constant(float(&mut args)?)
                } else {
                    DensityValue::Sampler(samplers[read::<usize>(&mut args)?].clone())
                };
                samplers.push(Arc::new(LerpSampler {
                    alpha,
                    first,
                    second,
                }));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "interval" => {
                let _specialization = read::<u8>(&mut args)?;
                let input = samplers[read::<usize>(&mut args)?].clone();
                let count = read::<usize>(&mut args)?;
                let thresholds = (0..count)
                    .map(|_| float(&mut args))
                    .collect::<Result<Vec<_>, _>>()?;
                let branches = (0..=count)
                    .map(|_| read::<usize>(&mut args).map(|id| samplers[id].clone()))
                    .collect::<Result<Vec<_>, _>>()?;
                samplers.push(Arc::new(IntervalSelectSampler::new(
                    input, thresholds, branches,
                )?));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "bounds" => {
                let mode = read::<u8>(&mut args)?;
                let a = float(&mut args)?;
                let b = float(&mut args)?;
                let c = float(&mut args)?;
                let d = float(&mut args)?;
                let e = float(&mut args)?;
                let f = float(&mut args)?;
                let result = match mode {
                    0 => Interval::new(a, b),
                    1 => Interval::symmetric(a),
                    2 => Interval::exact(a),
                    3 => Ok(Interval::encapsulating_values(a, b)),
                    4 => Ok(Interval::NAI),
                    5 => Ok(Interval::INFINITE),
                    _ => return Err("unknown interval constructor".into()),
                };
                interval_output(&mut out, "create", result)?;
                if let Ok(left) = result {
                    let right = Interval::encapsulating_values(c, d);
                    let third = Interval::encapsulating_values(e, f);
                    interval_output(&mut out, "right", Ok(right))?;
                    for (name, value) in [
                        ("add", left.add_range(right)),
                        ("sub", left.subtract_range(right)),
                        ("mul", left.multiply_range(right)),
                        ("div", left.divide_range(right)),
                        ("min", left.minimum(right)),
                        ("max", left.maximum(right)),
                        ("reciprocal", left.reciprocal()),
                        ("abs", left.abs()),
                        ("square", left.square()),
                        ("sign", left.sign()),
                        ("power", left.power(right)),
                    ] {
                        interval_output(&mut out, name, Ok(value))?;
                    }
                    interval_output(&mut out, "clamp", left.clamp(c, d))?;
                    interval_output(&mut out, "log", left.log())?;
                    interval_output(&mut out, "lerp-values", Ok(left.lerp_values(c, d)))?;
                    interval_output(&mut out, "lerp-ranges", Ok(left.lerp_ranges(right, third)))?;
                    interval_output(&mut out, "map-negate", left.map_monotonic(|v| -v))?;
                    interval_output(&mut out, "map-cube", left.map_monotonic(|v| v * v * v))?;
                    interval_output(
                        &mut out,
                        "map-sqrt",
                        left.map_monotonic(|v| (v as f64).sqrt() as f32),
                    )?;
                    interval_output(&mut out, "map-reciprocal", left.map_monotonic(|v| 1.0 / v))?;
                    interval_output(
                        &mut out,
                        "hull",
                        Interval::encapsulating(&[left, right, third]),
                    )?;
                    interval_output(&mut out, "empty", Interval::encapsulating(&[]))?;
                    writeln!(
                        out,
                        "bounds query {} {} {} {}",
                        u8::from(left.contains(e)),
                        u8::from(left.intersects(right)),
                        u8::from(left == right),
                        u8::from(left == result.unwrap())
                    )?;
                }
            }
            "offset" => {
                let kind = [
                    ShiftNoiseKind::Shift,
                    ShiftNoiseKind::ShiftA,
                    ShiftNoiseKind::ShiftB,
                ][read::<usize>(&mut args)?];
                let noise = noises[read::<usize>(&mut args)?].noise.clone();
                samplers.push(compile_shift_noise(noise, kind));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "blended" | "blendfail" => {
                let stream = args.next().ok_or("missing stream")?;
                let seed = read(&mut args)?;
                let definition = BlendedNoise {
                    xz_scale: double(&mut args)?,
                    y_scale: double(&mut args)?,
                    xz_factor: double(&mut args)?,
                    y_factor: double(&mut args)?,
                    smear_scale_multiplier: double(&mut args)?,
                };
                let result = if stream == "legacy" {
                    blended_probe(&mut LegacyRandom::new(seed), definition, &mut out)?
                } else {
                    blended_probe(&mut XoroshiroRandom::new(seed), definition, &mut out)?
                };
                if let Some(sampler) = result {
                    samplers.push(sampler);
                    writeln!(out, "create {}", samplers.len() - 1)?;
                }
            }
            "blendfbm" => {
                let stream = args.next().ok_or("missing stream")?;
                let seed = read(&mut args)?;
                let octave = read(&mut args)?;
                let smear = double(&mut args)?;
                let amplitude = double(&mut args)?;
                if stream == "legacy" {
                    fbm_probe(
                        &mut LegacyRandom::new(seed),
                        octave,
                        smear,
                        amplitude,
                        &mut out,
                    )?;
                } else {
                    fbm_probe(
                        &mut XoroshiroRandom::new(seed),
                        octave,
                        smear,
                        amplitude,
                        &mut out,
                    )?;
                }
            }
            "distance" => {
                let metric = [
                    DistanceMetric::Euclidean,
                    DistanceMetric::EuclideanSquared,
                    DistanceMetric::Manhattan,
                    DistanceMetric::Chebyshev,
                ][read::<usize>(&mut args)?];
                let point = [read(&mut args)?, read(&mut args)?, read(&mut args)?];
                samplers.push(Arc::new(DistanceToPointSampler { point, metric }));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "metric" => {
                let x = float(&mut args)?;
                let y = float(&mut args)?;
                let z = float(&mut args)?;
                for metric in [
                    DistanceMetric::Euclidean,
                    DistanceMetric::EuclideanSquared,
                    DistanceMetric::Manhattan,
                    DistanceMetric::Chebyshev,
                ] {
                    writeln!(
                        out,
                        "metric {} {}",
                        bits(metric.compute2(x, y)),
                        bits(metric.compute3(x, y, z))
                    )?;
                }
            }
            "wcontext" => {
                let cached = read::<u8>(&mut args)? != 0;
                let mut fields = WorldFields::default();
                for field in [
                    WorldField::BlendAlpha,
                    WorldField::BlendOffset,
                    WorldField::Beardifier,
                ] {
                    let binding = read::<isize>(&mut args)?;
                    if binding >= 0 {
                        fields = fields.with_field(field, samplers[binding as usize].clone());
                    }
                }
                if read::<u8>(&mut args)? != 0 {
                    fields = fields.with_blender(Arc::new(EmptyBlender));
                }
                context = (if cached {
                    SamplerContext::cached()
                } else {
                    SamplerContext::default()
                })
                .with_world_fields(fields);
                writeln!(out, "context")?;
            }
            "wfield" => {
                let field = [
                    WorldField::BlendAlpha,
                    WorldField::BlendOffset,
                    WorldField::Beardifier,
                ][read::<usize>(&mut args)?];
                let fallback = read::<isize>(&mut args)?;
                samplers.push(Arc::new(if fallback < 0 {
                    ContextBoundSampler::new(field)
                } else {
                    ContextBoundSampler::with_fallback(field, samplers[fallback as usize].clone())
                }));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "blenddensity" => {
                let input = samplers[read::<usize>(&mut args)?].clone();
                samplers.push(Arc::new(BlendDensitySampler { input }));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "context" => {
                context = if read::<u8>(&mut args)? == 0 {
                    SamplerContext::default()
                } else {
                    SamplerContext::cached()
                };
                writeln!(out, "context")?;
            }
            "counted" => {
                let input = samplers[read::<usize>(&mut args)?].clone();
                let calls = Arc::new(std::array::from_fn(|_| {
                    std::sync::atomic::AtomicUsize::new(0)
                }));
                counters.push(calls.clone());
                samplers.push(Arc::new(CountedSampler { input, calls }));
                writeln!(out, "counted {} {}", counters.len() - 1, samplers.len() - 1)?;
            }
            "counts" => {
                let calls = &counters[read::<usize>(&mut args)?];
                writeln!(
                    out,
                    "counts {} {}",
                    calls[0].load(std::sync::atomic::Ordering::Relaxed),
                    calls[1].load(std::sync::atomic::Ordering::Relaxed)
                )?;
            }
            "greset" => {
                graph_compiler = DensityCompiler::new(DensityGraph::new());
                functions.clear();
                graph_splines.clear();
                graph_noises.clear();
                context = SamplerContext::default();
                writeln!(out, "graph-reset")?;
            }
            "gseed" => {
                let seed = read::<i64>(&mut args)?;
                let legacy = read::<u8>(&mut args)? != 0;
                graph_compiler = DensityCompiler::new_seeded(DensityGraph::new(), seed, legacy);
                functions.clear();
                graph_splines.clear();
                graph_noises.clear();
                context = SamplerContext::default();
                writeln!(out, "graph-seed")?;
            }
            "gready" => writeln!(out, "graph-ready")?,
            "gend" => {
                functions.push(graph_compiler.graph_mut().insert(GraphNode::EndIslands)?);
                writeln!(out, "graph-create {}", functions.len() - 1)?;
            }
            "gregister" => {
                let name = args.next().ok_or("missing noise identifier")?.to_owned();
                let mut builder = NormalNoise::builder();
                builder
                    .set_base_octave(read(&mut args)?)
                    .set_base_amplitude(double(&mut args)?);
                builder.set_octave_count(read(&mut args)?)?;
                builder.set_normalization(
                    [
                        Normalization::Disabled,
                        Normalization::Enabled,
                        Normalization::Legacy,
                    ][read::<usize>(&mut args)?],
                );
                let modifiers = read::<usize>(&mut args)?;
                for index in 0..modifiers {
                    builder.set_amplitude_modifier(index as i32, double(&mut args)?)?;
                }
                graph_noises.push(
                    graph_compiler
                        .graph_mut()
                        .register_noise(name, builder.build()?)?,
                );
                writeln!(out, "graph-noise {}", graph_noises.len() - 1)?;
            }
            "ginstance" => {
                let id = graph_noises[read::<usize>(&mut args)?];
                let a = graph_compiler.get_noise_sampler(id)?;
                let b = graph_compiler.get_noise_sampler(id)?;
                writeln!(out, "graph-instance {}", u8::from(Arc::ptr_eq(&a, &b)))?;
            }
            "gbind" => {
                let sampler = samplers[read::<usize>(&mut args)?].clone();
                let range = Interval::encapsulating_values(float(&mut args)?, float(&mut args)?);
                let axes = read(&mut args)?;
                functions.push(graph_compiler.graph_mut().bind(sampler, range, axes)?);
                writeln!(out, "graph-create {}", functions.len() - 1)?;
            }
            "gsconstant" => {
                graph_splines.push(CubicSpline::Constant(float(&mut args)?));
                writeln!(out, "graph-spline {}", graph_splines.len() - 1)?;
            }
            "gspoints" => {
                let coordinate = functions[read::<usize>(&mut args)?];
                let count = read::<usize>(&mut args)?;
                let mut points = Vec::with_capacity(count);
                for _ in 0..count {
                    let location = float(&mut args)?;
                    let derivative = float(&mut args)?;
                    let value = graph_splines[read::<usize>(&mut args)?].clone();
                    points.push(SplinePoint {
                        location,
                        derivative,
                        value,
                    });
                }
                graph_splines.push(CubicSpline::multipoint(coordinate, points)?);
                writeln!(out, "graph-spline {}", graph_splines.len() - 1)?;
            }
            "gref" => {
                let input = functions[read::<usize>(&mut args)?];
                functions.push(graph_compiler.graph_mut().insert(GraphNode::Reference {
                    input,
                    holder: None,
                })?);
                writeln!(out, "graph-create {}", functions.len() - 1)?;
            }
            "gnamed" => {
                let name = read::<String>(&mut args)?;
                let input = functions[read::<usize>(&mut args)?];
                functions.push(graph_compiler.graph_mut().register_function(name, input)?);
                writeln!(out, "graph-create {}", functions.len() - 1)?;
            }
            "glookup" => {
                let name = read::<String>(&mut args)?;
                functions.push(
                    graph_compiler
                        .graph()
                        .function(&name)
                        .ok_or("unknown density name")?,
                );
                writeln!(out, "graph-create {}", functions.len() - 1)?;
            }
            "gspline" => {
                let spline = graph_splines[read::<usize>(&mut args)?].clone();
                functions.push(
                    graph_compiler
                        .graph_mut()
                        .insert(GraphNode::Spline(spline))?,
                );
                writeln!(out, "graph-create {}", functions.len() - 1)?;
            }
            command @ ("gconst" | "ggradient" | "gunary" | "gbinary" | "gclamp" | "gchoice"
            | "glerp" | "gslice" | "gcache" | "ginterp" | "gsurface" | "ginterval"
            | "gdistance" | "gnoise" | "goffset" | "gblended" | "ground" | "gpow"
            | "gwfield" | "gblenddensity") => {
                let reference = |args: &mut std::str::SplitWhitespace<'_>| -> Result<FunctionId,Box<dyn std::error::Error>> { Ok(functions[read::<usize>(args)?]) };
                let node = match command {
                    "gconst" => GraphNode::Constant(float(&mut args)?),
                    "gwfield" => GraphNode::WorldField(
                        [
                            WorldField::BlendAlpha,
                            WorldField::BlendOffset,
                            WorldField::Beardifier,
                        ][read::<usize>(&mut args)?],
                    ),
                    "gblenddensity" => GraphNode::BlendDensity(reference(&mut args)?),
                    "gpow" => GraphNode::Power {
                        base: reference(&mut args)?,
                        exponent: reference(&mut args)?,
                    },
                    "ground" => GraphNode::Round {
                        mode: [
                            RoundMode::Floor,
                            RoundMode::Round,
                            RoundMode::Ceil,
                            RoundMode::Truncate,
                        ][read::<usize>(&mut args)?],
                        input: reference(&mut args)?,
                        multiple: reference(&mut args)?,
                    },
                    "gnoise" => GraphNode::Noise {
                        noise: graph_noises[read::<usize>(&mut args)?],
                        xz_scale: double(&mut args)?,
                        y_scale: double(&mut args)?,
                        shift_x: reference(&mut args)?,
                        shift_y: reference(&mut args)?,
                        shift_z: reference(&mut args)?,
                    },
                    "goffset" => GraphNode::OffsetNoise {
                        noise: graph_noises[read::<usize>(&mut args)?],
                        kind: [
                            ShiftNoiseKind::Shift,
                            ShiftNoiseKind::ShiftA,
                            ShiftNoiseKind::ShiftB,
                        ][read::<usize>(&mut args)?],
                    },
                    "gblended" => GraphNode::BlendedNoise(BlendedNoise {
                        xz_scale: double(&mut args)?,
                        y_scale: double(&mut args)?,
                        xz_factor: double(&mut args)?,
                        y_factor: double(&mut args)?,
                        smear_scale_multiplier: double(&mut args)?,
                    }),
                    "gdistance" => GraphNode::DistanceToPoint {
                        point: [read(&mut args)?, read(&mut args)?, read(&mut args)?],
                        metric: [
                            DistanceMetric::Euclidean,
                            DistanceMetric::EuclideanSquared,
                            DistanceMetric::Manhattan,
                            DistanceMetric::Chebyshev,
                        ][read::<usize>(&mut args)?],
                    },
                    "ginterval" => {
                        let input = reference(&mut args)?;
                        let count: usize = read(&mut args)?;
                        let thresholds = (0..count)
                            .map(|_| float(&mut args))
                            .collect::<Result<Vec<_>, _>>()?;
                        let branches =
                            (0..=count)
                                .map(|_| reference(&mut args))
                                .collect::<Result<Vec<_>, _>>()?;
                        GraphNode::IntervalSelect {
                            input,
                            thresholds: thresholds.into(),
                            branches,
                        }
                    }
                    "ggradient" => {
                        let axis = [Axis::X, Axis::Y, Axis::Z][read::<usize>(&mut args)?];
                        let tiling = [
                            TilingMode::ClampToEdge,
                            TilingMode::Repeat,
                            TilingMode::MirroredRepeat,
                        ][read::<usize>(&mut args)?];
                        GraphNode::Gradient {
                            axis,
                            tiling,
                            from: read(&mut args)?,
                            to: read(&mut args)?,
                            low: float(&mut args)?,
                            high: float(&mut args)?,
                        }
                    }
                    "gunary" => {
                        let kind = [
                            GraphUnary::Abs,
                            GraphUnary::Square,
                            GraphUnary::Cube,
                            GraphUnary::HalfNegative,
                            GraphUnary::QuarterNegative,
                            GraphUnary::Reciprocal,
                            GraphUnary::Negate,
                            GraphUnary::Squeeze,
                            GraphUnary::Sign,
                            GraphUnary::Log,
                            GraphUnary::Sqrt,
                        ][read::<usize>(&mut args)?];
                        GraphNode::Unary {
                            kind,
                            input: reference(&mut args)?,
                        }
                    }
                    "gbinary" => {
                        let kind = [
                            GraphBinary::Add,
                            GraphBinary::Subtract,
                            GraphBinary::Multiply,
                            GraphBinary::Divide,
                            GraphBinary::Minimum,
                            GraphBinary::Maximum,
                        ][read::<usize>(&mut args)?];
                        GraphNode::Binary {
                            kind,
                            left: reference(&mut args)?,
                            right: reference(&mut args)?,
                        }
                    }
                    "gclamp" => GraphNode::Clamp {
                        input: reference(&mut args)?,
                        min: float(&mut args)?,
                        max: float(&mut args)?,
                    },
                    "gchoice" => GraphNode::Choice {
                        input: reference(&mut args)?,
                        min: float(&mut args)?,
                        max: float(&mut args)?,
                        inside: reference(&mut args)?,
                        outside: reference(&mut args)?,
                    },
                    "glerp" => GraphNode::Lerp {
                        alpha: reference(&mut args)?,
                        first: reference(&mut args)?,
                        second: reference(&mut args)?,
                    },
                    "gslice" => GraphNode::Slice {
                        axis: [Axis::X, Axis::Y, Axis::Z][read::<usize>(&mut args)?],
                        coordinate: read(&mut args)?,
                        input: reference(&mut args)?,
                    },
                    "gcache" => GraphNode::Cache(reference(&mut args)?),
                    "ginterp" => GraphNode::Interpolated {
                        input: reference(&mut args)?,
                        xz: read(&mut args)?,
                        y: read(&mut args)?,
                    },
                    "gsurface" => GraphNode::Surface {
                        density: reference(&mut args)?,
                        upper: reference(&mut args)?,
                        lower: read(&mut args)?,
                        height: read(&mut args)?,
                    },
                    _ => unreachable!(),
                };
                functions.push(graph_compiler.graph_mut().insert(node)?);
                writeln!(out, "graph-create {}", functions.len() - 1)?;
            }
            "ginfo" => {
                let id = functions[read::<usize>(&mut args)?];
                let range = graph_compiler
                    .graph()
                    .range(id)
                    .map_err(|_| IntervalError::NanBound);
                interval_output(&mut out, "graph", range)?;
                writeln!(
                    out,
                    "graph-axes {}",
                    graph_compiler.graph().domain_axes(id)?
                )?;
            }
            "gsame" => {
                let a = functions[read::<usize>(&mut args)?];
                let b = functions[read::<usize>(&mut args)?];
                writeln!(
                    out,
                    "graph-same {}",
                    u8::from(graph_compiler.graph().java_equals(a, b)?)
                )?;
            }
            command @ ("gcompile" | "gtry") => {
                let mode = read::<u8>(&mut args)?;
                let id = functions[read::<usize>(&mut args)?];
                let result = if mode == 0 {
                    graph_compiler.compile_raw(id)
                } else {
                    graph_compiler.get_sampler(id)
                };
                if command == "gtry" {
                    writeln!(
                        out,
                        "graph-try {} {}",
                        if result.is_ok() { "ok" } else { "error" },
                        graph_compiler.cache_count()
                    )?;
                } else {
                    let sampler = result?;
                    let same = if mode == 0 {
                        -1
                    } else {
                        i32::from(Arc::ptr_eq(&sampler, &graph_compiler.get_sampler(id)?))
                    };
                    samplers.push(sampler);
                    writeln!(
                        out,
                        "graph-compile {} {} {}",
                        samplers.len() - 1,
                        graph_compiler.cache_count(),
                        same
                    )?;
                }
            }
            "spconstant" => {
                splines.push(CubicSpline::Constant(float(&mut args)?));
                writeln!(out, "spline {}", splines.len() - 1)?;
            }
            "sppoints" => {
                let coordinate = read(&mut args)?;
                let count = read::<usize>(&mut args)?;
                let mut points = Vec::new();
                for _ in 0..count {
                    let location = float(&mut args)?;
                    let derivative = float(&mut args)?;
                    let value = splines[read::<usize>(&mut args)?].clone();
                    points.push(SplinePoint {
                        location,
                        derivative,
                        value,
                    });
                }
                splines.push(CubicSpline::multipoint(coordinate, points)?);
                writeln!(out, "spline {}", splines.len() - 1)?;
            }
            "spfail" => {
                let spline = &splines[read::<usize>(&mut args)?];
                writeln!(
                    out,
                    "spline-compile {}",
                    if SplineSampler::new(spline, &samplers).is_err() {
                        "error"
                    } else {
                        "ok"
                    }
                )?;
            }
            "sprange" => {
                let spline = &splines[read::<usize>(&mut args)?];
                let mut ranges = [Interval::NAI; 3];
                for range in &mut ranges {
                    *range = Interval::encapsulating_values(float(&mut args)?, float(&mut args)?);
                }
                interval_output(&mut out, "spline", spline.range(&mut |id| ranges[id % 3]))?;
            }
            "spsampler" => {
                let spline = &splines[read::<usize>(&mut args)?];
                samplers.push(Arc::new(SplineSampler::new(spline, &samplers)?));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "surface" => {
                let mode = read::<u8>(&mut args)?;
                let density = samplers[read::<usize>(&mut args)?].clone();
                let upper = samplers[read::<usize>(&mut args)?].clone();
                let lower = read(&mut args)?;
                let height = read(&mut args)?;
                let sampler: Arc<dyn DensitySampler> = if mode == 0 {
                    Arc::new(FindTopSurfaceSampler::new(density, upper, lower, height)?)
                } else {
                    Arc::new(compile_surface_search(density, upper, lower, height)?)
                };
                samplers.push(sampler);
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "interpolate" => {
                let input = samplers[read::<usize>(&mut args)?].clone();
                let xz = read(&mut args)?;
                let y = read(&mut args)?;
                samplers.push(Arc::new(InterpolatedSampler::new(input, xz, y)?));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "slice" => {
                let mode = read::<u8>(&mut args)?;
                let input = samplers[read::<usize>(&mut args)?].clone();
                let a = read(&mut args)?;
                let b = read(&mut args)?;
                let axes = match mode {
                    0 => SliceAxes::X(a),
                    1 => SliceAxes::Y(a),
                    2 => SliceAxes::Z(a),
                    3 => SliceAxes::Xz { x: a, z: b },
                    _ => return Err("invalid slice mode".into()),
                };
                samplers.push(Arc::new(SliceSampler { input, axes }));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "cache" => {
                let id = read::<usize>(&mut args)?;
                let input = samplers[read::<usize>(&mut args)?].clone();
                samplers.push(Arc::new(CachingDensitySampler::new(id, input)?));
                writeln!(out, "create {}", samplers.len() - 1)?;
            }
            "lookup" => {
                let sizes = [read(&mut args)?, read(&mut args)?, read(&mut args)?];
                let minimum = [read(&mut args)?, read(&mut args)?, read(&mut args)?];
                let steps = [read(&mut args)?, read(&mut args)?, read(&mut args)?];
                let position = [read(&mut args)?, read(&mut args)?, read(&mut args)?];
                let volume = DensityVolume::new(sizes, minimum, steps)?;
                writeln!(
                    out,
                    "lookup {}",
                    volume.index_of_block(position).map_or(-1, |v| v as i32)
                )?;
            }
            "query" => {
                let sampler = &samplers[read::<usize>(&mut args)?];
                let position = [read(&mut args)?, read(&mut args)?, read(&mut args)?];
                match sampler.sample_value(&mut context, position) {
                    Ok(value) => writeln!(out, "query {}", bits(value))?,
                    Err(_) => writeln!(out, "query error")?,
                }
            }
            "volume" => {
                let sampler = &samplers[read::<usize>(&mut args)?];
                let sizes = [read(&mut args)?, read(&mut args)?, read(&mut args)?];
                let minimum = [read(&mut args)?, read(&mut args)?, read(&mut args)?];
                let steps = [read(&mut args)?, read(&mut args)?, read(&mut args)?];
                let tail = read::<usize>(&mut args)?;
                let seed = read(&mut args)?;
                let repeats = read::<usize>(&mut args)?;
                let volume = DensityVolume::new(sizes, minimum, steps)?;
                let mut random = LegacyRandom::new(seed);
                let mut buffer: Vec<_> = (0..volume.size() + tail)
                    .map(|_| f32::from_bits(random.next_int() as u32))
                    .collect();
                for pass in 0..repeats {
                    let result = sampler.sample_volume(&mut context, &mut buffer, volume);
                    writeln!(
                        out,
                        "volume {pass} {}",
                        if result.is_ok() { "ok" } else { "error" }
                    )?;
                    for (i, value) in buffer.iter().enumerate() {
                        writeln!(out, "cell {i} {}", bits(*value))?;
                    }
                }
            }
            _ => return Err("unknown command".into()),
        }
        if args.next().is_some() {
            return Err("extra argument".into());
        }
    }
    out.flush()?;
    Ok(())
}
