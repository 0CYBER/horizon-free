use super::*;

/// Vanilla's three density fields, resolved at evaluation time rather than
/// during graph compilation. Their declared ranges do not inspect bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldField {
    BlendAlpha,
    BlendOffset,
    Beardifier,
}
impl WorldField {
    pub fn domain_axes(self) -> u8 {
        match self {
            Self::BlendAlpha | Self::BlendOffset => 5,
            Self::Beardifier => 7,
        }
    }
    pub fn range(self) -> Interval {
        match self {
            Self::BlendAlpha => Interval::new(0.0, 1.0).unwrap(),
            Self::BlendOffset | Self::Beardifier => Interval::INFINITE,
        }
    }
    fn fallback(self) -> f32 {
        match self {
            Self::BlendAlpha => 1.0,
            Self::BlendOffset | Self::Beardifier => 0.0,
        }
    }
}

/// Old-world blending provider. The full vanilla Blender data model and
/// distance-weighting algorithm are a separate, still pending implementation.
pub trait DensityBlender: std::fmt::Debug + Send + Sync {
    fn is_empty(&self) -> bool;
    fn blend_density(&self, position: [i32; 3], input: f32) -> f32;
}

#[derive(Debug)]
pub struct EmptyBlender;
impl DensityBlender for EmptyBlender {
    fn is_empty(&self) -> bool {
        true
    }
    fn blend_density(&self, _: [i32; 3], input: f32) -> f32 {
        input
    }
}

/// Bindings can be assembled and shared between workers. Attaching new
/// bindings consumes the worker context and invalidates its cached densities.
#[derive(Clone, Debug, Default)]
pub struct WorldFields {
    samplers: [Option<Arc<dyn DensitySampler>>; 3],
    blender: Option<Arc<dyn DensityBlender>>,
}
impl WorldFields {
    pub fn with_field(mut self, field: WorldField, sampler: Arc<dyn DensitySampler>) -> Self {
        self.samplers[field as usize] = Some(sampler);
        self
    }
    pub fn with_blender(mut self, blender: Arc<dyn DensityBlender>) -> Self {
        self.blender = Some(blender);
        self
    }
}
impl SamplerContext {
    pub fn with_world_fields(mut self, fields: WorldFields) -> Self {
        self.clear_caches();
        self.fields = fields;
        self
    }
}

#[derive(Clone, Debug)]
pub struct ContextBoundSampler {
    field: WorldField,
    fallback: Arc<dyn DensitySampler>,
}
impl ContextBoundSampler {
    pub fn new(field: WorldField) -> Self {
        Self::with_fallback(field, Arc::new(ConstantSampler(field.fallback())))
    }
    /// The vanilla sampler also permits a fallback supplied by its caller.
    pub fn with_fallback(field: WorldField, fallback: Arc<dyn DensitySampler>) -> Self {
        Self { field, fallback }
    }
    fn resolve(&self, context: &SamplerContext) -> Arc<dyn DensitySampler> {
        context.fields.samplers[self.field as usize]
            .as_ref()
            .unwrap_or(&self.fallback)
            .clone()
    }
}
impl DensitySampler for ContextBoundSampler {
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, SampleError> {
        self.resolve(context).sample_value(context, position)
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        self.resolve(context).sample_volume(context, buffer, volume)
    }
}

#[derive(Clone, Debug)]
pub struct BlendDensitySampler {
    pub input: Arc<dyn DensitySampler>,
}
impl DensitySampler for BlendDensitySampler {
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, SampleError> {
        let input = self.input.sample_value(context, position)?;
        let Some(blender) = &context.fields.blender else {
            return Ok(input);
        };
        Ok(if blender.is_empty() {
            input
        } else {
            blender.blend_density(position, input)
        })
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        self.input.sample_volume(context, buffer, volume)?;
        let Some(blender) = &context.fields.blender else {
            return Ok(());
        };
        if blender.is_empty() {
            return Ok(());
        }
        visit(volume, |index, position| {
            buffer[index] = blender.blend_density(position, buffer[index]);
            Ok(())
        })
    }
}
