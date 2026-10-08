use horizon_core::{
    density::{
        BlendDensitySampler, CachingDensitySampler, ConstantSampler, ContextBoundSampler,
        DensityBlender, DensitySampler, SampleError, SamplerContext, WorldField, WorldFields,
        graph::{DensityCompiler, DensityGraph, GraphNode},
    },
    noise::DensityVolume,
};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct SpatialBlender {
    calls: Mutex<Vec<Option<[i32; 3]>>>,
}
impl DensityBlender for SpatialBlender {
    fn is_empty(&self) -> bool {
        self.calls.lock().unwrap().push(None);
        false
    }
    fn blend_density(&self, [x, y, z]: [i32; 3], input: f32) -> f32 {
        self.calls.lock().unwrap().push(Some([x, y, z]));
        input + (x * 100 + y * 10 + z) as f32
    }
}

#[test]
fn fields_resolve_through_the_same_context_and_replace_cached_values() {
    let alpha: Arc<dyn DensitySampler> = Arc::new(ContextBoundSampler::new(WorldField::BlendAlpha));
    let beard = ContextBoundSampler::new(WorldField::Beardifier);
    let fields = WorldFields::default()
        .with_field(WorldField::Beardifier, alpha)
        .with_field(WorldField::BlendAlpha, Arc::new(ConstantSampler(0.25)));
    let mut context = SamplerContext::cached().with_world_fields(fields);
    let cached = CachingDensitySampler::new(0, Arc::new(beard)).unwrap();
    assert_eq!(cached.sample_value(&mut context, [0; 3]).unwrap(), 0.25);
    context = context.with_world_fields(
        WorldFields::default().with_field(WorldField::Beardifier, Arc::new(ConstantSampler(0.75))),
    );
    assert_eq!(cached.sample_value(&mut context, [0; 3]).unwrap(), 0.75);
}

#[test]
fn absent_fields_use_vanilla_defaults_including_buffer_tails() {
    let volume = DensityVolume::new([1; 3], [0; 3], [1; 3]).unwrap();
    let mut context = SamplerContext::default();
    for (field, expected) in [
        (WorldField::BlendAlpha, 1.0),
        (WorldField::BlendOffset, 0.0),
        (WorldField::Beardifier, 0.0),
    ] {
        let mut buffer = [9.0; 3];
        ContextBoundSampler::new(field)
            .sample_volume(&mut context, &mut buffer, volume)
            .unwrap();
        assert_eq!(buffer, [expected; 3]);
    }
}

#[test]
fn blending_visits_z_x_y_and_preserves_the_input_tail() {
    let blender = Arc::new(SpatialBlender {
        calls: Mutex::new(Vec::new()),
    });
    let mut context = SamplerContext::default()
        .with_world_fields(WorldFields::default().with_blender(blender.clone()));
    let sampler = BlendDensitySampler {
        input: Arc::new(ConstantSampler(0.5)),
    };
    let volume = DensityVolume::new([2; 3], [1, 2, 3], [2, 3, 4]).unwrap();
    let mut buffer = [9.0; 10];
    sampler
        .sample_volume(&mut context, &mut buffer, volume)
        .unwrap();
    assert_eq!(
        *blender.calls.lock().unwrap(),
        vec![
            None,
            Some([1, 2, 3]),
            Some([1, 5, 3]),
            Some([3, 2, 3]),
            Some([3, 5, 3]),
            Some([1, 2, 7]),
            Some([1, 5, 7]),
            Some([3, 2, 7]),
            Some([3, 5, 7])
        ]
    );
    assert_eq!(
        buffer,
        [
            123.5, 153.5, 323.5, 353.5, 127.5, 157.5, 327.5, 357.5, 0.5, 0.5
        ]
    );
}

#[derive(Debug)]
struct FailedInput;
impl DensitySampler for FailedInput {
    fn sample_value(&self, _: &mut SamplerContext, _: [i32; 3]) -> Result<f32, SampleError> {
        Err(SampleError::ZeroGradientPeriod)
    }
    fn sample_volume(
        &self,
        _: &mut SamplerContext,
        buffer: &mut [f32],
        _: DensityVolume,
    ) -> Result<(), SampleError> {
        buffer[0] = 0.25;
        Err(SampleError::ZeroGradientPeriod)
    }
}
#[test]
fn failed_input_prevents_blender_lookup_and_preserves_partial_writes() {
    let blender = Arc::new(SpatialBlender {
        calls: Mutex::new(Vec::new()),
    });
    let mut context = SamplerContext::default()
        .with_world_fields(WorldFields::default().with_blender(blender.clone()));
    let sampler = BlendDensitySampler {
        input: Arc::new(FailedInput),
    };
    assert!(sampler.sample_value(&mut context, [0; 3]).is_err());
    let mut buffer = [9.0; 2];
    assert!(
        sampler
            .sample_volume(
                &mut context,
                &mut buffer,
                DensityVolume::new([1; 3], [0; 3], [1; 3]).unwrap()
            )
            .is_err()
    );
    assert_eq!(buffer, [0.25, 9.0]);
    assert!(blender.calls.lock().unwrap().is_empty());
}

#[test]
fn blending_range_and_axes_follow_input_even_for_spatial_providers() {
    let mut graph = DensityGraph::default();
    let constant = graph.insert(GraphNode::Constant(0.5)).unwrap();
    let root = graph.insert(GraphNode::BlendDensity(constant)).unwrap();
    assert_eq!(graph.domain_axes(root).unwrap(), 0);
    assert_eq!(graph.range(root).unwrap(), graph.range(constant).unwrap());
    let mut compiler = DensityCompiler::new(graph);
    let raw = compiler.compile_raw(root).unwrap();
    let optimized = compiler.get_sampler(root).unwrap();
    let blender = Arc::new(SpatialBlender {
        calls: Mutex::new(Vec::new()),
    });
    let mut context =
        SamplerContext::default().with_world_fields(WorldFields::default().with_blender(blender));
    assert_eq!(raw.sample_value(&mut context, [1, 2, 3]).unwrap(), 123.5);
    assert_eq!(
        optimized.sample_value(&mut context, [1, 2, 3]).unwrap(),
        0.5
    );
}

#[test]
fn named_reference_identity_differs_from_direct_value_equality() {
    let mut graph = DensityGraph::new();
    let value = graph.insert(GraphNode::Constant(-0.0)).unwrap();
    let direct = graph
        .insert(GraphNode::Reference {
            input: value,
            holder: None,
        })
        .unwrap();
    let a = graph
        .register_function("minecraft:a".into(), value)
        .unwrap();
    let b = graph
        .register_function("minecraft:b".into(), value)
        .unwrap();
    assert!(!graph.java_equals(a, b).unwrap());
    assert!(!graph.java_equals(a, direct).unwrap());
    assert_eq!(graph.function("minecraft:a"), Some(a));
    assert!(
        graph
            .register_function("minecraft:a".into(), value)
            .is_err()
    );
    let exponent = graph.insert(GraphNode::Constant(0.5)).unwrap();
    let power = graph
        .insert(GraphNode::Power { base: a, exponent })
        .unwrap();
    let mut compiler = DensityCompiler::new(graph);
    let raw = compiler.compile_raw(power).unwrap();
    let optimized = compiler.get_sampler(power).unwrap();
    let mut context = SamplerContext::default();
    assert_eq!(
        raw.sample_value(&mut context, [0; 3]).unwrap().to_bits(),
        0x8000_0000
    );
    assert_eq!(
        optimized
            .sample_value(&mut context, [0; 3])
            .unwrap()
            .to_bits(),
        0
    );
}
