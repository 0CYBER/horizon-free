//! The eight terrain-router fields used by Java Edition 26.3.
use super::{DecodeError, Object, climate::ClimateSampler, density::DensityRegistry, required};
use horizon_core::density::{
    DensitySampler,
    graph::{CompileError, DensityCompiler, FunctionId},
};
use std::sync::Arc;

/// Declaration and compilation order: the six climate fields, surface, density.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum RouterField {
    Temperature,
    Vegetation,
    Continents,
    Erosion,
    Depth,
    Ridges,
    ChunkSurfaceLevel,
    FinalDensity,
}
impl RouterField {
    pub const ALL: [Self; 8] = [
        Self::Temperature,
        Self::Vegetation,
        Self::Continents,
        Self::Erosion,
        Self::Depth,
        Self::Ridges,
        Self::ChunkSurfaceLevel,
        Self::FinalDensity,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Temperature => "temperature",
            Self::Vegetation => "vegetation",
            Self::Continents => "continents",
            Self::Erosion => "erosion",
            Self::Depth => "depth",
            Self::Ridges => "ridges",
            Self::ChunkSurfaceLevel => "chunk_surface_level",
            Self::FinalDensity => "final_density",
        }
    }
}

/// Root IDs belong to the registry graph used to decode this router.
#[derive(Clone, Copy, Debug)]
pub struct NoiseRouter {
    functions: [FunctionId; 8],
}
impl NoiseRouter {
    /// Decode the router object itself. Noise settings, aquifer and material
    /// configuration are separate codecs and are not parsed by this method.
    /// Missing/null fields or any invalid root leave the registry unchanged.
    pub fn from_json(json: &str, registry: &mut DensityRegistry) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let inputs = RouterField::ALL
            .iter()
            .map(|field| required(&object, field.name()).map(|raw| raw.get()))
            .collect::<Result<Vec<_>, _>>()?;
        let functions = registry
            .decode_functions(&inputs)?
            .try_into()
            .map_err(|_| DecodeError::Field("router field count"))?;
        Ok(Self { functions })
    }
    pub fn function(self, field: RouterField) -> FunctionId {
        self.functions[field as usize]
    }
    /// One compiler owns the cache namespace for every router field. Existing
    /// compiler samplers and cache IDs are reused rather than reset per root.
    pub fn compile(self, compiler: &mut DensityCompiler) -> Result<RouterSamplers, CompileError> {
        let samplers = self
            .functions
            .iter()
            .map(|id| compiler.get_sampler(*id))
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .expect("one sampler for each of eight router fields");
        Ok(RouterSamplers { samplers })
    }
    /// Compile only the six climate roots, in vanilla's sampler-binding order.
    pub fn compile_climate(
        self,
        compiler: &mut DensityCompiler,
    ) -> Result<ClimateSampler, CompileError> {
        let samplers = self.functions[..6]
            .iter()
            .map(|id| compiler.get_sampler(*id))
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .expect("one sampler for each of six climate fields");
        Ok(ClimateSampler::new(samplers))
    }
}
#[derive(Debug)]
pub struct RouterSamplers {
    samplers: [Arc<dyn DensitySampler>; 8],
}
impl RouterSamplers {
    pub fn get(&self, field: RouterField) -> &Arc<dyn DensitySampler> {
        &self.samplers[field as usize]
    }
    pub fn climate_sampler(&self) -> ClimateSampler {
        ClimateSampler::new(std::array::from_fn(|i| self.samplers[i].clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::density::SamplerContext;
    use std::collections::BTreeMap;

    #[test]
    fn roots_share_holder_cache_and_failed_router_leaves_graph_unchanged() {
        let mut registry = DensityRegistry::from_json(
            &BTreeMap::new(),
            &BTreeMap::from([(
                "minecraft:shared".into(),
                r#"{"type":"cache","input":{"type":"gradient","axis":"x","from_coordinate":0,"to_coordinate":16,"from_value":-1,"to_value":1}}"#.into(),
            )]),
        )
        .unwrap();
        let object: BTreeMap<_, _> = RouterField::ALL
            .iter()
            .map(|field| (field.name(), "minecraft:shared"))
            .collect();
        let json = serde_json::to_string(&object).unwrap();
        let expected = registry
            .graph()
            .clone()
            .insert(horizon_core::density::graph::GraphNode::Constant(123.0))
            .unwrap();
        let invalid = json
            .replace(
                "\"temperature\":\"minecraft:shared\"",
                "\"temperature\":123",
            )
            .replace(
                "\"final_density\":\"minecraft:shared\"",
                "\"final_density\":{\"type\":\"missing\"}",
            );
        assert!(NoiseRouter::from_json(&invalid, &mut registry).is_err());
        assert_eq!(registry.decode_functions(&["123"]).unwrap()[0], expected);
        let router = NoiseRouter::from_json(&json, &mut registry).unwrap();
        for field in RouterField::ALL {
            assert_eq!(
                router.function(field),
                registry.reference("minecraft:shared").unwrap()
            );
        }
        let mut compiler = DensityCompiler::new_seeded(registry.into_graph(), 0, false);
        let samplers = router.compile(&mut compiler).unwrap();
        assert_eq!(compiler.cache_count(), 1);
        let mut context = SamplerContext::cached();
        for field in RouterField::ALL {
            assert_eq!(
                samplers
                    .get(field)
                    .sample_value(&mut context, [4, 99, -32])
                    .unwrap(),
                -0.5
            );
        }
    }
}
