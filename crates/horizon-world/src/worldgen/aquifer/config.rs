//! The six density roots in the 26.3 aquifer configuration.
use super::super::{DecodeError, Object, density::DensityRegistry, required};
use horizon_core::density::{
    DensitySampler,
    graph::{CompileError, DensityCompiler, FunctionId},
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum AquiferField {
    Barrier,
    Floodedness,
    Spread,
    Lava,
    Exclusion,
    SurfaceLevel,
}
impl AquiferField {
    pub const ALL: [Self; 6] = [
        Self::Barrier,
        Self::Floodedness,
        Self::Spread,
        Self::Lava,
        Self::Exclusion,
        Self::SurfaceLevel,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Barrier => "barrier",
            Self::Floodedness => "fluid_level_floodedness",
            Self::Spread => "fluid_level_spread",
            Self::Lava => "lava",
            Self::Exclusion => "exclusion",
            Self::SurfaceLevel => "surface_level",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct AquiferConfig {
    functions: [FunctionId; 6],
}
impl AquiferConfig {
    /// Decode all roots atomically into the caller's registry graph.
    pub fn from_json(json: &str, registry: &mut DensityRegistry) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let inputs = AquiferField::ALL
            .iter()
            .map(|f| required(&object, f.name()).map(|v| v.get()))
            .collect::<Result<Vec<_>, _>>()?;
        let functions = registry
            .decode_functions(&inputs)?
            .try_into()
            .map_err(|_| DecodeError::Field("aquifer field count"))?;
        Ok(Self { functions })
    }
    pub fn function(self, field: AquiferField) -> FunctionId {
        self.functions[field as usize]
    }
    /// Preserve vanilla's binding order and the shared compiler cache namespace.
    pub fn compile(self, compiler: &mut DensityCompiler) -> Result<AquiferSamplers, CompileError> {
        let samplers = self
            .functions
            .iter()
            .map(|id| compiler.get_sampler(*id))
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .expect("six aquifer roots");
        Ok(AquiferSamplers { samplers })
    }
}
#[derive(Debug)]
pub struct AquiferSamplers {
    samplers: [Arc<dyn DensitySampler>; 6],
}
impl AquiferSamplers {
    /// Bind already compiled roots in AquiferField::ALL order.
    pub fn new(samplers: [Arc<dyn DensitySampler>; 6]) -> Self {
        Self { samplers }
    }
    pub fn get(&self, field: AquiferField) -> &Arc<dyn DensitySampler> {
        &self.samplers[field as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::density::{SamplerContext, graph::GraphNode};
    use std::collections::BTreeMap;
    #[test]
    fn bindings_share_named_cache_and_failed_decode_is_atomic() {
        let mut registry = DensityRegistry::from_json(&BTreeMap::new(), &BTreeMap::from([("minecraft:shared".into(), r#"{"type":"cache","input":{"type":"gradient","axis":"x","from_coordinate":0,"to_coordinate":16,"from_value":0,"to_value":1}}"#.into())])).unwrap();
        let roots: BTreeMap<_, _> = AquiferField::ALL
            .iter()
            .map(|f| (f.name(), "minecraft:shared"))
            .collect();
        let json = serde_json::to_string(&roots).unwrap();
        let expected = registry
            .graph()
            .clone()
            .insert(GraphNode::Constant(123.))
            .unwrap();
        let invalid = json
            .replace("\"barrier\":\"minecraft:shared\"", "\"barrier\":123")
            .replace(
                "\"surface_level\":\"minecraft:shared\"",
                "\"surface_level\":{}",
            );
        assert!(AquiferConfig::from_json(&invalid, &mut registry).is_err());
        assert_eq!(registry.decode_functions(&["123"]).unwrap()[0], expected);
        let config = AquiferConfig::from_json(&json, &mut registry).unwrap();
        let mut compiler = DensityCompiler::new_seeded(registry.into_graph(), 0, false);
        let samplers = config.compile(&mut compiler).unwrap();
        assert_eq!(compiler.cache_count(), 1);
        let mut context = SamplerContext::cached();
        for field in AquiferField::ALL {
            assert_eq!(
                samplers
                    .get(field)
                    .sample_value(&mut context, [4, -54, 0])
                    .unwrap(),
                0.25
            );
        }
    }
}
