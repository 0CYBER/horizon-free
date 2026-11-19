//! Bounded decoding of density registries into the immutable core graph.
//! Named registry holders and raw numeric tokens preserve compilation behavior.
use super::mod_density_functions::{
    MAX_REASONABLE, ModDensity, ModDensityFunctionTypes, mod_density_function_types,
};
use super::*;
use horizon_core::{
    density::{
        BlendedNoise, CubicSpline, DistanceMetric, Interval, RoundMode, ShiftNoiseKind,
        SplinePoint, TilingMode, WorldField,
        graph::{DensityGraph, FunctionId, GraphBinary, GraphNode, GraphUnary, NoiseId},
    },
    geometry::Axis,
};
use std::{collections::BTreeSet, sync::Arc};

pub(super) fn identifier(name: &str) -> Result<String, DecodeError> {
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let namespace = if namespace.is_empty() {
        "minecraft"
    } else {
        namespace
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    if !namespace.bytes().all(valid)
        || !path.bytes().all(|c| valid(c) || c == b'/')
        || path.is_empty()
    {
        return Err(DecodeError::Registry(format!(
            "invalid registry identifier: {name}"
        )));
    }
    Ok(format!("{namespace}:{path}"))
}
fn string(raw: &RawValue) -> Result<String, DecodeError> {
    Ok(serde_json::from_str(raw.get())?)
}
fn float(raw: &RawValue) -> Result<f32, DecodeError> {
    number_text(raw)?
        .parse()
        .map_err(|_| DecodeError::Field("float"))
}
fn noise_value(raw: &RawValue) -> Result<f32, DecodeError> {
    let value = float(raw)?;
    if (-MAX_REASONABLE..=MAX_REASONABLE).contains(&value) {
        Ok(value)
    } else {
        Err(DecodeError::Field("density value"))
    }
}
fn axis(raw: &RawValue) -> Result<Axis, DecodeError> {
    match string(raw)?.as_str() {
        "x" => Ok(Axis::X),
        "y" => Ok(Axis::Y),
        "z" => Ok(Axis::Z),
        _ => Err(DecodeError::Field("axis")),
    }
}

/// The mod density function types a registry's JSON may name.
#[derive(Clone, Copy)]
struct ModTypes(&'static ModDensityFunctionTypes);
impl std::fmt::Debug for ModTypes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

/// All entries are loaded atomically. Named child references resolve through
/// distinct registry holders; returned entry values are their direct graphs.
#[derive(Debug)]
pub struct DensityRegistry {
    graph: DensityGraph,
    values: BTreeMap<String, FunctionId>,
    noises: BTreeMap<String, NoiseId>,
    types: ModTypes,
}
impl DensityRegistry {
    /// Input keys are registry identifiers, and values contain original JSON.
    /// This API accepts acyclic registries, at most 256 nested graph levels and
    /// at most 256 interval branches. Inline noise holders are not yet supported.
    /// A `type` vanilla doesn't have is one of the installed mod density function types.
    pub fn from_json(
        noises: &BTreeMap<String, String>,
        densities: &BTreeMap<String, String>,
    ) -> Result<Self, DecodeError> {
        Self::from_json_with_types(noises, densities, mod_density_function_types())
    }
    /// [`Self::from_json`] with `types` in place of the installed mod types.
    pub(crate) fn from_json_with_types(
        noises: &BTreeMap<String, String>,
        densities: &BTreeMap<String, String>,
        types: &'static ModDensityFunctionTypes,
    ) -> Result<Self, DecodeError> {
        let mut loader = Loader {
            types: ModTypes(types),
            graph: DensityGraph::new(),
            noises: BTreeMap::new(),
            definitions: BTreeMap::new(),
            values: BTreeMap::new(),
            active: BTreeSet::new(),
        };
        for (name, json) in noises {
            let name = identifier(name)?;
            let noise = loader
                .graph
                .register_noise(name.clone(), decode_noise(json)?)?;
            loader.noises.insert(name, noise);
        }
        for (name, json) in densities {
            let name = identifier(name)?;
            let raw: Box<RawValue> = serde_json::from_str(json)?;
            if loader.definitions.insert(name.clone(), raw).is_some() {
                return Err(DecodeError::Registry(format!(
                    "duplicate density identifier: {name}"
                )));
            }
        }
        let names: Vec<_> = loader.definitions.keys().cloned().collect();
        for name in names {
            loader.resolve(&name, 0)?;
        }
        Ok(Self {
            graph: loader.graph,
            values: loader.values,
            noises: loader.noises,
            types: loader.types,
        })
    }
    pub fn graph(&self) -> &DensityGraph {
        &self.graph
    }
    pub fn into_graph(self) -> DensityGraph {
        self.graph
    }
    pub fn entries(&self) -> impl Iterator<Item = (&str, FunctionId)> {
        self.values.iter().map(|(name, id)| (name.as_str(), *id))
    }
    pub fn value(&self, name: &str) -> Option<FunctionId> {
        self.values.get(name).copied()
    }
    pub fn reference(&self, name: &str) -> Option<FunctionId> {
        self.graph.function(name)
    }
    /// Decode direct roots against this registry, retaining shared named holders.
    /// All roots are added atomically; a failure leaves existing IDs and graphs
    /// intact. Roots are not registered as additional named density entries.
    pub fn decode_functions(&mut self, json: &[&str]) -> Result<Vec<FunctionId>, DecodeError> {
        let mut loader = Loader {
            types: self.types,
            graph: self.graph.clone(),
            noises: self.noises.clone(),
            definitions: BTreeMap::new(),
            values: BTreeMap::new(),
            active: BTreeSet::new(),
        };
        let roots = json
            .iter()
            .map(|json| {
                let raw: Box<RawValue> = serde_json::from_str(json)?;
                loader.decode(&raw, 0)
            })
            .collect::<Result<_, _>>()?;
        self.graph = loader.graph;
        Ok(roots)
    }
}
struct Loader {
    types: ModTypes,
    graph: DensityGraph,
    noises: BTreeMap<String, NoiseId>,
    definitions: BTreeMap<String, Box<RawValue>>,
    values: BTreeMap<String, FunctionId>,
    active: BTreeSet<String>,
}
impl Loader {
    fn resolve(&mut self, name: &str, depth: usize) -> Result<FunctionId, DecodeError> {
        if let Some(id) = self.graph.function(name) {
            return Ok(id);
        }
        if depth > 256 {
            return Err(DecodeError::Field("density nesting"));
        }
        if !self.active.insert(name.to_owned()) {
            return Err(DecodeError::Registry(format!(
                "cyclic density reference: {name}"
            )));
        }
        let raw = self
            .definitions
            .get(name)
            .ok_or_else(|| DecodeError::Registry(format!("unknown density reference: {name}")))?
            .clone();
        let value = self.decode(&raw, depth + 1)?;
        let reference = self.graph.register_function(name.to_owned(), value)?;
        self.values.insert(name.to_owned(), value);
        self.active.remove(name);
        Ok(reference)
    }
    fn input(
        &mut self,
        object: &Object,
        name: &'static str,
        depth: usize,
    ) -> Result<FunctionId, DecodeError> {
        self.decode(required(object, name)?, depth + 1)
    }
    fn optional_input(
        &mut self,
        object: &Object,
        name: &'static str,
        fallback: f32,
        depth: usize,
    ) -> Result<FunctionId, DecodeError> {
        match optional(object, name) {
            Some(raw) => self.decode(raw, depth + 1),
            None => Ok(self.graph.insert(GraphNode::Constant(fallback))?),
        }
    }
    fn noise(&self, object: &Object) -> Result<NoiseId, DecodeError> {
        let raw = required(object, "noise")?;
        if !raw.get().starts_with('"') {
            return Err(DecodeError::Field("inline noise holder unsupported"));
        }
        let name = identifier(&string(raw)?)?;
        self.noises
            .get(&name)
            .copied()
            .ok_or_else(|| DecodeError::Registry(format!("unknown noise reference: {name}")))
    }
    fn decode(&mut self, raw: &RawValue, depth: usize) -> Result<FunctionId, DecodeError> {
        if depth > 256 {
            return Err(DecodeError::Field("density nesting"));
        }
        if raw.get().starts_with('"') {
            return self.resolve(&identifier(&string(raw)?)?, depth);
        }
        if !raw.get().starts_with('{') {
            return Ok(self.graph.insert(GraphNode::Constant(noise_value(raw)?))?);
        }
        let object: Object = serde_json::from_str(raw.get())?;
        let kind = identifier(&string(required(&object, "type")?)?)?;
        let unary = match kind.as_str() {
            "minecraft:abs" => Some(GraphUnary::Abs),
            "minecraft:square" => Some(GraphUnary::Square),
            "minecraft:cube" => Some(GraphUnary::Cube),
            "minecraft:sqrt" => Some(GraphUnary::Sqrt),
            "minecraft:half_negative" => Some(GraphUnary::HalfNegative),
            "minecraft:quarter_negative" => Some(GraphUnary::QuarterNegative),
            "minecraft:reciprocal" => Some(GraphUnary::Reciprocal),
            "minecraft:negate" => Some(GraphUnary::Negate),
            "minecraft:squeeze" => Some(GraphUnary::Squeeze),
            "minecraft:log" => Some(GraphUnary::Log),
            "minecraft:sign" => Some(GraphUnary::Sign),
            _ => None,
        };
        let binary = match kind.as_str() {
            "minecraft:add" => Some(GraphBinary::Add),
            "minecraft:sub" => Some(GraphBinary::Subtract),
            "minecraft:mul" => Some(GraphBinary::Multiply),
            "minecraft:div" => Some(GraphBinary::Divide),
            "minecraft:min" => Some(GraphBinary::Minimum),
            "minecraft:max" => Some(GraphBinary::Maximum),
            _ => None,
        };
        let node = if let Some(kind) = unary {
            GraphNode::Unary {
                kind,
                input: self.input(&object, "input", depth)?,
            }
        } else if let Some(kind) = binary {
            GraphNode::Binary {
                kind,
                left: self.input(&object, "left", depth)?,
                right: self.input(&object, "right", depth)?,
            }
        } else {
            match kind.as_str() {
                "minecraft:constant" => {
                    GraphNode::Constant(noise_value(required(&object, "value")?)?)
                }
                "minecraft:blend_alpha" => GraphNode::WorldField(WorldField::BlendAlpha),
                "minecraft:blend_offset" => GraphNode::WorldField(WorldField::BlendOffset),
                "minecraft:beardifier" => GraphNode::WorldField(WorldField::Beardifier),
                "minecraft:end_outer_islands" => GraphNode::EndIslands,
                "minecraft:cache" => GraphNode::Cache(self.input(&object, "input", depth)?),
                "minecraft:blend_density" => {
                    GraphNode::BlendDensity(self.input(&object, "input", depth)?)
                }
                "minecraft:pow" => GraphNode::Power {
                    base: self.input(&object, "base", depth)?,
                    exponent: self.input(&object, "exponent", depth)?,
                },
                "minecraft:floor" | "minecraft:round" | "minecraft:ceil" | "minecraft:truncate" => {
                    GraphNode::Round {
                        mode: match kind.as_str() {
                            "minecraft:floor" => RoundMode::Floor,
                            "minecraft:round" => RoundMode::Round,
                            "minecraft:ceil" => RoundMode::Ceil,
                            _ => RoundMode::Truncate,
                        },
                        input: self.input(&object, "input", depth)?,
                        multiple: self.optional_input(&object, "multiple", 1.0, depth)?,
                    }
                }
                "minecraft:gradient" => {
                    let from = integer(required(&object, "from_coordinate")?)?;
                    let to = integer(required(&object, "to_coordinate")?)?;
                    if from == to {
                        return Err(DecodeError::Field("gradient coordinates"));
                    }
                    GraphNode::Gradient {
                        axis: axis(required(&object, "axis")?)?,
                        tiling: match optional(&object, "tiling")
                            .map(string)
                            .transpose()?
                            .as_deref()
                            .unwrap_or("clamp_to_edge")
                        {
                            "clamp_to_edge" => TilingMode::ClampToEdge,
                            "repeat" => TilingMode::Repeat,
                            "mirrored_repeat" => TilingMode::MirroredRepeat,
                            _ => return Err(DecodeError::Field("tiling")),
                        },
                        from,
                        to,
                        low: noise_value(required(&object, "from_value")?)?,
                        high: noise_value(required(&object, "to_value")?)?,
                    }
                }
                "minecraft:noise" => GraphNode::Noise {
                    noise: self.noise(&object)?,
                    xz_scale: double(required(&object, "xz_scale")?)?,
                    y_scale: double(required(&object, "y_scale")?)?,
                    shift_x: self.optional_input(&object, "shift_x", 0.0, depth)?,
                    shift_y: self.optional_input(&object, "shift_y", 0.0, depth)?,
                    shift_z: self.optional_input(&object, "shift_z", 0.0, depth)?,
                },
                "minecraft:shift" | "minecraft:shift_a" | "minecraft:shift_b" => {
                    GraphNode::OffsetNoise {
                        noise: self.noise(&object)?,
                        kind: match kind.as_str() {
                            "minecraft:shift" => ShiftNoiseKind::Shift,
                            "minecraft:shift_a" => ShiftNoiseKind::ShiftA,
                            _ => ShiftNoiseKind::ShiftB,
                        },
                    }
                }
                "minecraft:old_blended_noise" => GraphNode::BlendedNoise(BlendedNoise {
                    xz_scale: double_range(
                        required(&object, "xz_scale")?,
                        "xz_scale",
                        0.001,
                        1000.0,
                    )?,
                    y_scale: double_range(required(&object, "y_scale")?, "y_scale", 0.001, 1000.0)?,
                    xz_factor: double_range(
                        required(&object, "xz_factor")?,
                        "xz_factor",
                        0.001,
                        1000.0,
                    )?,
                    y_factor: double_range(
                        required(&object, "y_factor")?,
                        "y_factor",
                        0.001,
                        1000.0,
                    )?,
                    smear_scale_multiplier: double_range(
                        required(&object, "smear_scale_multiplier")?,
                        "smear_scale_multiplier",
                        1.0,
                        8.0,
                    )?,
                }),
                "minecraft:slice" => GraphNode::Slice {
                    axis: axis(required(&object, "axis")?)?,
                    coordinate: integer(required(&object, "coordinate")?)?,
                    input: self.input(&object, "input", depth)?,
                },
                "minecraft:interpolated" => GraphNode::Interpolated {
                    input: self.input(&object, "input", depth)?,
                    xz: int_range(
                        required(&object, "cell_size_xz")?,
                        "cell_size_xz",
                        1,
                        i32::MAX,
                    )?,
                    y: int_range(
                        required(&object, "cell_size_y")?,
                        "cell_size_y",
                        1,
                        i32::MAX,
                    )?,
                },
                "minecraft:find_top_surface" => GraphNode::Surface {
                    density: self.input(&object, "density", depth)?,
                    upper: self.input(&object, "upper_bound", depth)?,
                    lower: int_range(
                        required(&object, "lower_bound")?,
                        "lower_bound",
                        -4064,
                        4062,
                    )?,
                    height: int_range(
                        required(&object, "cell_height")?,
                        "cell_height",
                        1,
                        i32::MAX,
                    )?,
                },
                "minecraft:clamp" => {
                    let min = noise_value(required(&object, "min")?)?;
                    let max = noise_value(required(&object, "max")?)?;
                    if max < min {
                        return Err(DecodeError::Field("clamp bounds"));
                    }
                    GraphNode::Clamp {
                        input: self.input(&object, "input", depth)?,
                        min,
                        max,
                    }
                }
                "minecraft:range_choice" => GraphNode::Choice {
                    input: self.input(&object, "input", depth)?,
                    min: noise_value(required(&object, "min_inclusive")?)?,
                    max: noise_value(required(&object, "max_exclusive")?)?,
                    inside: self.input(&object, "when_in_range", depth)?,
                    outside: self.input(&object, "when_out_of_range", depth)?,
                },
                "minecraft:lerp" => GraphNode::Lerp {
                    alpha: self.input(&object, "alpha", depth)?,
                    first: self.input(&object, "first", depth)?,
                    second: self.input(&object, "second", depth)?,
                },
                "minecraft:interval_select" => {
                    let thresholds: Vec<Box<RawValue>> =
                        serde_json::from_str(required(&object, "thresholds")?.get())?;
                    let thresholds: Vec<f32> = thresholds
                        .iter()
                        .map(|raw| noise_value(raw))
                        .collect::<Result<_, _>>()?;
                    let functions: Vec<Box<RawValue>> =
                        serde_json::from_str(required(&object, "functions")?.get())?;
                    if functions.len() < 2
                        || functions.len() > 256
                        || functions.len() != thresholds.len() + 1
                        || thresholds
                            .windows(2)
                            .any(|pair| pair[0].total_cmp(&pair[1]).is_gt())
                    {
                        return Err(DecodeError::Field("interval selection"));
                    }
                    let input = self.input(&object, "input", depth)?;
                    let branches = functions
                        .iter()
                        .map(|raw| self.decode(raw, depth + 1))
                        .collect::<Result<_, _>>()?;
                    GraphNode::IntervalSelect {
                        input,
                        thresholds: Arc::from(thresholds),
                        branches,
                    }
                }
                "minecraft:distance_to_point" => {
                    let raw: Vec<Box<RawValue>> =
                        serde_json::from_str(required(&object, "point")?.get())?;
                    if raw.len() != 3 {
                        return Err(DecodeError::Field("point"));
                    }
                    let point = [integer(&raw[0])?, integer(&raw[1])?, integer(&raw[2])?];
                    let metric = match string(required(&object, "metric")?)?.as_str() {
                        "euclidean" => DistanceMetric::Euclidean,
                        "euclidean_squared" => DistanceMetric::EuclideanSquared,
                        "manhattan" => DistanceMetric::Manhattan,
                        "chebyshev" => DistanceMetric::Chebyshev,
                        _ => return Err(DecodeError::Field("metric")),
                    };
                    GraphNode::DistanceToPoint { point, metric }
                }
                "minecraft:spline" => {
                    GraphNode::Spline(self.spline(required(&object, "spline")?, depth + 1)?)
                }
                _ => return self.mod_density(&kind, raw),
            }
        };
        Ok(self.graph.insert(node)?)
    }
    /// A node of the mod density function type `kind`: the whole object goes to its decoder, and
    /// the function it gives is bound to the world seed when the graph compiles.
    fn mod_density(&mut self, kind: &str, raw: &RawValue) -> Result<FunctionId, DecodeError> {
        let refused = |why: String| DecodeError::Registry(format!("density type {kind}: {why}"));
        let Some(density_type) = self.types.0.get(kind) else {
            return Err(DecodeError::Registry(format!(
                "unknown density type: {kind}"
            )));
        };
        let function = density_type.decode(raw.get()).map_err(refused)?;
        let [min, max] = function.range();
        let range = Interval::new(min, max)
            .ok()
            .filter(|_| min.is_finite() && max.is_finite())
            .ok_or_else(|| {
                refused(format!(
                    "range [{min}, {max}] is not two finite values, the least first"
                ))
            })?;
        if min < -MAX_REASONABLE || max > MAX_REASONABLE {
            return Err(refused(format!(
                "range [{min}, {max}] goes beyond ±{MAX_REASONABLE}, the most a density value may be"
            )));
        }
        // The axis mask the compiler slices on: X=1, Y=2, Z=4.
        let axes = if function.varies_with_y() { 7 } else { 5 };
        let node = ModDensity {
            id: kind.to_owned(),
            owner: density_type.owner().to_owned(),
            range: [min, max],
            function,
        };
        Ok(self.graph.bind_seeded(Arc::new(node), range, axes)?)
    }
    fn spline(
        &mut self,
        raw: &RawValue,
        depth: usize,
    ) -> Result<CubicSpline<FunctionId>, DecodeError> {
        if depth > 256 {
            return Err(DecodeError::Field("spline nesting"));
        }
        if !raw.get().starts_with('{') {
            return Ok(CubicSpline::Constant(float(raw)?));
        }
        let object: Object = serde_json::from_str(raw.get())?;
        let coordinate = self.input(&object, "coordinate", depth)?;
        let points: Vec<Box<RawValue>> = serde_json::from_str(required(&object, "points")?.get())?;
        let points = points
            .iter()
            .map(|raw| {
                let point: Object = serde_json::from_str(raw.get())?;
                Ok(SplinePoint {
                    location: float(required(&point, "location")?)?,
                    derivative: float(required(&point, "derivative")?)?,
                    value: self.spline(required(&point, "value")?, depth + 1)?,
                })
            })
            .collect::<Result<_, DecodeError>>()?;
        CubicSpline::multipoint(coordinate, points)
            .map_err(|error| DecodeError::Graph(error.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::density::{SamplerContext, graph::DensityCompiler};
    #[test]
    fn numeric_tokens_keep_negative_zero_and_avoid_double_rounding() {
        let densities = BTreeMap::from([
            ("minecraft:negative_zero".into(), "-0".into()),
            (
                "minecraft:rounded".into(),
                "1.0000000596046447753906250000000001".into(),
            ),
        ]);
        let registry = DensityRegistry::from_json(&BTreeMap::new(), &densities).unwrap();
        let zero = registry.value("minecraft:negative_zero").unwrap();
        let rounded = registry.value("minecraft:rounded").unwrap();
        let compiler = DensityCompiler::new(registry.into_graph());
        let mut context = SamplerContext::default();
        assert_eq!(
            compiler
                .compile_raw(zero)
                .unwrap()
                .sample_value(&mut context, [0; 3])
                .unwrap()
                .to_bits(),
            0x8000_0000
        );
        assert_eq!(
            compiler
                .compile_raw(rounded)
                .unwrap()
                .sample_value(&mut context, [0; 3])
                .unwrap()
                .to_bits(),
            0x3f80_0001
        );
    }
    #[test]
    fn forward_references_share_named_holders_and_reject_cycles_or_missing_values() {
        let mut densities = BTreeMap::from([
            (
                "minecraft:a".into(),
                r#"{"type":"add","left":"z","right":"minecraft:z"}"#.into(),
            ),
            ("minecraft:z".into(), "0.5".into()),
        ]);
        let registry = DensityRegistry::from_json(&BTreeMap::new(), &densities).unwrap();
        let root = registry.value("minecraft:a").unwrap();
        let compiler = DensityCompiler::new(registry.into_graph());
        assert_eq!(
            compiler
                .compile_raw(root)
                .unwrap()
                .sample_value(&mut SamplerContext::default(), [0; 3])
                .unwrap(),
            1.0
        );
        densities.insert("minecraft:z".into(), r#""a""#.into());
        assert!(DensityRegistry::from_json(&BTreeMap::new(), &densities).is_err());
        densities.remove("minecraft:z");
        assert!(DensityRegistry::from_json(&BTreeMap::new(), &densities).is_err());
    }
    #[test]
    fn codec_validation_rejects_invalid_gradients_intervals_and_splines() {
        for json in [
            r#"{"type":"gradient","axis":"y","from_coordinate":0,"to_coordinate":0,"from_value":0,"to_value":1}"#,
            r#"{"type":"interval_select","input":0,"thresholds":[0,-0],"functions":[0,1,2]}"#,
            r#"{"type":"interpolated","input":0,"cell_size_xz":0,"cell_size_y":8}"#,
            r#"{"type":"spline","spline":{"coordinate":0,"points":[]}}"#,
            r#"{"type":"clamp","input":0,"min":1,"max":-1}"#,
        ] {
            assert!(
                DensityRegistry::from_json(
                    &BTreeMap::new(),
                    &BTreeMap::from([("minecraft:a".into(), json.into())])
                )
                .is_err(),
                "{json}"
            );
        }
    }

    mod mod_types {
        use super::super::super::mod_density_functions::{
            ModDensityFunction, ModDensityFunctionType, ModDensityFunctionTypes,
        };
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// `{"type": "test:ramp", "per_y": k}`: k times y plus the seed, in [-1000, 1000].
        struct Ramp;
        struct Ramped(f32);
        impl ModDensityFunctionType for Ramp {
            fn owner(&self) -> &str {
                "test"
            }
            fn decode(&self, json: &str) -> Result<Arc<dyn ModDensityFunction>, String> {
                let value: serde_json::Value =
                    serde_json::from_str(json).map_err(|e| e.to_string())?;
                let per_y = value["per_y"].as_f64().ok_or("needs per_y")?;
                Ok(Arc::new(Ramped(per_y as f32)))
            }
        }
        impl ModDensityFunction for Ramped {
            fn range(&self) -> [f32; 2] {
                [-1000.0, 1000.0]
            }
            fn varies_with_y(&self) -> bool {
                true
            }
            fn sample(&self, [_, y, _]: [i32; 3], seed: i64) -> f32 {
                self.0 * y as f32 + seed as f32
            }
            fn fill(&self, _: [i32; 3], _: [i32; 3], _: [usize; 3], _: i64, _: &mut [f32]) {
                unreachable!("these tests sample points")
            }
        }
        /// A type whose functions declare `range`.
        struct Ranged([f32; 2]);
        impl ModDensityFunctionType for Ranged {
            fn owner(&self) -> &str {
                "test"
            }
            fn decode(&self, _: &str) -> Result<Arc<dyn ModDensityFunction>, String> {
                Ok(Arc::new(Ranged(self.0)))
            }
        }
        impl ModDensityFunction for Ranged {
            fn range(&self) -> [f32; 2] {
                self.0
            }
            fn varies_with_y(&self) -> bool {
                false
            }
            fn sample(&self, _: [i32; 3], _: i64) -> f32 {
                self.0[0]
            }
            fn fill(&self, _: [i32; 3], _: [i32; 3], _: [usize; 3], _: i64, _: &mut [f32]) {}
        }

        /// In [-10, 10], 1 everywhere, except where x > 0: there `fill` leaves slot 0 alone
        /// (`None`) or writes the value into it, and `sample` returns that value (NaN for `None`).
        struct Misbehaves(Option<f32>);
        impl ModDensityFunctionType for Misbehaves {
            fn owner(&self) -> &str {
                "test"
            }
            fn decode(&self, _: &str) -> Result<Arc<dyn ModDensityFunction>, String> {
                Ok(Arc::new(Misbehaves(self.0)))
            }
        }
        impl ModDensityFunction for Misbehaves {
            fn range(&self) -> [f32; 2] {
                [-10.0, 10.0]
            }
            fn varies_with_y(&self) -> bool {
                true
            }
            fn sample(&self, [x, _, _]: [i32; 3], _: i64) -> f32 {
                if x > 0 {
                    self.0.unwrap_or(f32::NAN)
                } else {
                    1.0
                }
            }
            fn fill(&self, origin: [i32; 3], _: [i32; 3], _: [usize; 3], _: i64, out: &mut [f32]) {
                for (slot, value) in out.iter_mut().enumerate() {
                    match (slot, origin[0] > 0, self.0) {
                        (0, true, None) => {}
                        (0, true, Some(bad)) => *value = bad,
                        _ => *value = 1.0,
                    }
                }
            }
        }

        fn types() -> &'static ModDensityFunctionTypes {
            Box::leak(Box::new(ModDensityFunctionTypes::from([
                ("test:skips".to_owned(), Arc::new(Misbehaves(None)) as _),
                (
                    "test:steady".to_owned(),
                    Arc::new(Misbehaves(Some(1.0))) as _,
                ),
                (
                    "test:wild".to_owned(),
                    Arc::new(Misbehaves(Some(20.0))) as _,
                ),
                (
                    "test:huge".to_owned(),
                    Arc::new(Ranged([0.0, 4_294_967_296.0])) as _,
                ),
                ("test:ramp".to_owned(), Arc::new(Ramp) as _),
                (
                    "test:inverted".to_owned(),
                    Arc::new(Ranged([1.0, -1.0])) as _,
                ),
                (
                    "test:endless".to_owned(),
                    Arc::new(Ranged([0.0, f32::INFINITY])) as _,
                ),
            ])))
        }
        fn decode(json: &str) -> Result<DensityRegistry, DecodeError> {
            DensityRegistry::from_json_with_types(
                &BTreeMap::new(),
                &BTreeMap::from([("minecraft:a".into(), json.into())]),
                types(),
            )
        }

        /// A registered type decodes with its own JSON fields, composes with vanilla nodes and
        /// takes the world seed when a seeded compiler compiles it; its range feeds the graph's.
        #[test]
        fn a_registered_type_decodes_its_fields_and_takes_the_compilers_seed() {
            let registry =
                decode(r#"{"type":"add","left":{"type":"test:ramp","per_y":0.5},"right":2}"#)
                    .unwrap();
            let root = registry.value("minecraft:a").unwrap();
            assert_eq!(
                registry.graph().range(root).unwrap(),
                Interval::new(-998.0, 1002.0).unwrap()
            );
            assert_eq!(registry.graph().domain_axes(root).unwrap(), 7);
            let graph = registry.into_graph();
            let seeded = DensityCompiler::new_seeded(graph.clone(), 100, false);
            let sampler = seeded.compile_raw(root).unwrap();
            let value = sampler.sample_value(&mut SamplerContext::default(), [7, 10, -3]);
            assert_eq!(value.unwrap(), 0.5 * 10.0 + 100.0 + 2.0);
            // Without a seed there is nothing to bind it to.
            assert!(DensityCompiler::new(graph).compile_raw(root).is_err());
        }

        /// The message of the panic `f` raises.
        fn panic_message(f: impl FnOnce()) -> String {
            let payload =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).expect_err("panics");
            payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default()
        }

        /// A slot `fill` leaves unwritten panics, naming the mod, the type and the slot, though
        /// the same scratch held the first volume's values under `add`; so does a NaN or
        /// out-of-range value from `fill` or `sample`. None is used, cached or turned into an
        /// error.
        #[test]
        fn unwritten_nan_and_out_of_range_values_panic_naming_the_mod_and_type() {
            let densities = BTreeMap::from([
                (
                    "minecraft:skips".into(),
                    r#"{"type":"add","left":{"type":"test:skips"},"right":{"type":"test:steady"}}"#
                        .into(),
                ),
                ("minecraft:wild".into(), r#"{"type":"test:wild"}"#.into()),
            ]);
            let registry =
                DensityRegistry::from_json_with_types(&BTreeMap::new(), &densities, types())
                    .unwrap();
            let skips = registry.value("minecraft:skips").unwrap();
            let wild = registry.value("minecraft:wild").unwrap();
            let mut compiler = DensityCompiler::new_seeded(registry.into_graph(), 4, false);
            let sampler = compiler.get_sampler(skips).unwrap();
            let mut context = SamplerContext::default();
            let mut values = vec![0.0; 12];
            let volume = |x| {
                horizon_core::noise::DensityVolume::new([2, 3, 2], [x, 0, 0], [4, 16, 4]).unwrap()
            };
            sampler
                .sample_volume(&mut context, &mut values, volume(-8))
                .unwrap();
            assert!(values.iter().all(|&value| value == 2.0), "{values:?}");
            let message = panic_message(|| {
                let _ = sampler.sample_volume(&mut context, &mut values, volume(8));
            });
            assert_eq!(
                message,
                "native mod test density type test:skips left slot 0 unwritten or wrote NaN"
            );
            let message = panic_message(|| {
                let _ = sampler.sample_value(&mut SamplerContext::default(), [5, 0, 0]);
            });
            assert_eq!(
                message,
                "native mod test density type test:skips returned NaN at [5, 0, 0]"
            );
            let wild = compiler.get_sampler(wild).unwrap();
            let message = panic_message(|| {
                let _ = wild.sample_volume(&mut SamplerContext::default(), &mut values, volume(8));
            });
            assert_eq!(
                message,
                "native mod test density type test:wild wrote 20 at slot 0, outside its declared range [-10, 10]"
            );
            let message = panic_message(|| {
                let _ = wild.sample_value(&mut SamplerContext::default(), [5, 0, 0]);
            });
            assert_eq!(
                message,
                "native mod test density type test:wild returned 20 at [5, 0, 0], outside its declared range [-10, 10]"
            );
        }

        /// A type nobody registered is still refused by name, a mod decoder's refusal names the
        /// type and its message, and a range that isn't two finite ordered values is refused.
        #[test]
        fn unknown_types_decoder_refusals_and_bad_ranges_are_errors_naming_the_type() {
            let error = |json: &str| decode(json).unwrap_err().to_string();
            assert!(
                error(r#"{"type":"test:gone"}"#).contains("unknown density type: test:gone"),
                "{}",
                error(r#"{"type":"test:gone"}"#)
            );
            assert!(
                error(r#"{"type":"test:ramp"}"#).contains("density type test:ramp: needs per_y")
            );
            assert!(
                error(r#"{"type":"test:inverted"}"#).contains("density type test:inverted: range")
            );
            assert!(
                error(r#"{"type":"test:endless"}"#).contains("density type test:endless: range")
            );
            // Beyond vanilla's reasonable bound, `find_top_surface`'s cell steps would overflow.
            assert!(
                error(r#"{"type":"test:huge"}"#).contains("goes beyond ±1000000"),
                "{}",
                error(r#"{"type":"test:huge"}"#)
            );
            // The installed table never holds this module's types.
            let densities = BTreeMap::from([(
                "minecraft:a".into(),
                r#"{"type":"test:ramp","per_y":1}"#.into(),
            )]);
            assert!(DensityRegistry::from_json(&BTreeMap::new(), &densities).is_err());
        }

        static POINTS: AtomicUsize = AtomicUsize::new(0);
        static VOLUMES: AtomicUsize = AtomicUsize::new(0);
        /// `x + 10 z + seed`, plus `100 y` when it varies with y, counting how it is asked.
        struct Counted {
            varies: bool,
        }
        impl Counted {
            fn value(&self, [x, y, z]: [i32; 3], seed: i64) -> f32 {
                let y = if self.varies { 100 * y } else { 0 };
                (x + 10 * z + y) as f32 + seed as f32
            }
        }
        impl ModDensityFunctionType for Counted {
            fn owner(&self) -> &str {
                "test"
            }
            fn decode(&self, _: &str) -> Result<Arc<dyn ModDensityFunction>, String> {
                Ok(Arc::new(Counted {
                    varies: self.varies,
                }))
            }
        }
        impl ModDensityFunction for Counted {
            fn range(&self) -> [f32; 2] {
                [-1e6, 1e6]
            }
            fn varies_with_y(&self) -> bool {
                self.varies
            }
            fn sample(&self, position: [i32; 3], seed: i64) -> f32 {
                POINTS.fetch_add(1, Ordering::Relaxed);
                self.value(position, seed)
            }
            fn fill(
                &self,
                origin: [i32; 3],
                step: [i32; 3],
                count: [usize; 3],
                seed: i64,
                out: &mut [f32],
            ) {
                VOLUMES.fetch_add(1, Ordering::Relaxed);
                assert_eq!(out.len(), count.iter().product::<usize>());
                let mut index = 0;
                for z in 0..count[2] as i32 {
                    for x in 0..count[0] as i32 {
                        for y in 0..count[1] as i32 {
                            let at = [x, y, z];
                            let position = std::array::from_fn(|a| origin[a] + at[a] * step[a]);
                            out[index] = self.value(position, seed);
                            index += 1;
                        }
                    }
                }
            }
        }

        /// Nested under vanilla nodes and through the compiler's own passes (a y-free node is
        /// sliced at y = 0), a mod node is asked for whole volumes, never point by point, and
        /// gives the values its `sample` would.
        #[test]
        fn a_nested_mod_node_is_filled_by_volume_with_and_without_y() {
            let types: &'static ModDensityFunctionTypes =
                Box::leak(Box::new(ModDensityFunctionTypes::from([
                    (
                        "test:with_y".to_owned(),
                        Arc::new(Counted { varies: true }) as _,
                    ),
                    (
                        "test:without_y".to_owned(),
                        Arc::new(Counted { varies: false }) as _,
                    ),
                ])));
            let densities = BTreeMap::from([
                (
                    "minecraft:with_y".into(),
                    r#"{"type":"add","left":{"type":"test:with_y"},"right":2}"#.into(),
                ),
                (
                    "minecraft:without_y".into(),
                    r#"{"type":"clamp","input":{"type":"test:without_y"},"min":-1e6,"max":1e6}"#
                        .into(),
                ),
            ]);
            let registry =
                DensityRegistry::from_json_with_types(&BTreeMap::new(), &densities, types).unwrap();
            let roots = [
                (registry.value("minecraft:with_y").unwrap(), true, 2.0),
                (registry.value("minecraft:without_y").unwrap(), false, 0.0),
            ];
            let seed = 9;
            let mut compiler = DensityCompiler::new_seeded(registry.into_graph(), seed, false);
            let volume =
                horizon_core::noise::DensityVolume::new([3, 5, 2], [-7, 16, 4], [4, 8, 4]).unwrap();
            for (root, varies, offset) in roots {
                let sampler = compiler.get_sampler(root).unwrap();
                let mut values = vec![0.0; volume.size()];
                sampler
                    .sample_volume(&mut SamplerContext::default(), &mut values, volume)
                    .unwrap();
                let function = Counted { varies };
                let mut index = 0;
                for z in 0..2 {
                    for x in 0..3 {
                        for y in 0..5 {
                            let position = [-7 + 4 * x, 16 + 8 * y, 4 + 4 * z];
                            let expected = function.value(position, seed) + offset;
                            assert_eq!(values[index], expected, "{position:?} with y: {varies}");
                            index += 1;
                        }
                    }
                }
            }
            assert_eq!(POINTS.load(Ordering::Relaxed), 0);
            assert!(VOLUMES.load(Ordering::Relaxed) >= 2);
        }

        /// The vanilla registry (the prepared one where it is present, a small one of vanilla
        /// node types otherwise) decodes and samples bit for bit the same with mod types
        /// installed as without.
        #[test]
        fn vanilla_densities_are_unchanged_by_installed_mod_types() {
            let root = std::env::var_os("HORIZON_TEST_RESOURCES")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../.local/horizon-client/26.3")
                });
            let read = |name: &str| -> Option<BTreeMap<String, String>> {
                let text = std::fs::read_to_string(root.join("worldgen").join(name)).ok()?;
                serde_json::from_str(&text).ok()
            };
            let (noises, densities) = match (read("noises.json"), read("densities.json")) {
                (Some(noises), Some(densities)) => (noises, densities),
                _ => (
                    BTreeMap::new(),
                    BTreeMap::from([
                        (
                            "minecraft:ramp".into(),
                            r#"{"type":"gradient","axis":"y","from_coordinate":-64,"to_coordinate":320,"from_value":1.5,"to_value":-1.5}"#.into(),
                        ),
                        (
                            "minecraft:shaped".into(),
                            r#"{"type":"clamp","input":{"type":"mul","left":"minecraft:ramp","right":{"type":"distance_to_point","point":[3,0,-5],"metric":"manhattan"}},"min":-64,"max":64}"#.into(),
                        ),
                    ]),
                ),
            };
            let empty: &'static ModDensityFunctionTypes =
                Box::leak(Box::new(ModDensityFunctionTypes::new()));
            let samples = |types| {
                let registry =
                    DensityRegistry::from_json_with_types(&noises, &densities, types).unwrap();
                let names: Vec<(String, FunctionId)> = registry
                    .entries()
                    .map(|(name, id)| (name.to_owned(), id))
                    .collect();
                let mut compiler = DensityCompiler::new_seeded(registry.into_graph(), 12345, false);
                let volume = horizon_core::noise::DensityVolume::new(
                    [2, 6, 2],
                    [-21, -40, 37],
                    [16, 24, 16],
                )
                .unwrap();
                names
                    .into_iter()
                    .map(|(name, id)| {
                        let mut values = vec![0.0f32; volume.size()];
                        // Some entries sample only inside a chunk's context: compare their errors.
                        let sampled = compiler
                            .get_sampler(id)
                            .map_err(|e| e.to_string())
                            .and_then(|sampler| {
                                sampler
                                    .sample_volume(
                                        &mut SamplerContext::cached(),
                                        &mut values,
                                        volume,
                                    )
                                    .map_err(|e| e.to_string())
                            });
                        let bits: Vec<u32> = values.iter().map(|v| v.to_bits()).collect();
                        (name, sampled, bits)
                    })
                    .collect::<Vec<_>>()
            };
            let without = samples(empty);
            assert!(!without.is_empty());
            assert_eq!(without, samples(types()));
        }
    }
}
