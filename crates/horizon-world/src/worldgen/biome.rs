//! Multi-noise biome resolution over an imported immutable parameter table.
//! Vanilla biome JSON codecs and built-in preset builders remain separate work.
use super::{
    DecodeError,
    climate::{
        ClimateIndex, ClimateSampler, ClimateSearch, ClimateVolume, IndexError, Parameter,
        ParameterPoint,
    },
    density::identifier,
};
use horizon_core::density::{SampleError, SamplerContext};

/// Identity within an immutable biome registry snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BiomeId(pub u32);

#[derive(Debug)]
pub enum BiomeError {
    Decode(DecodeError),
    Index(IndexError),
    Sample(SampleError),
    OutsideVolume,
}
impl std::fmt::Display for BiomeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(e) => e.fmt(f),
            Self::Index(e) => e.fmt(f),
            Self::Sample(e) => e.fmt(f),
            Self::OutsideVolume => f.write_str("biome query is outside the prepared quart volume"),
        }
    }
}
impl std::error::Error for BiomeError {}
impl From<DecodeError> for BiomeError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}
impl From<IndexError> for BiomeError {
    fn from(e: IndexError) -> Self {
        Self::Index(e)
    }
}
impl From<SampleError> for BiomeError {
    fn from(e: SampleError) -> Self {
        Self::Sample(e)
    }
}

#[derive(Debug)]
pub struct MultiNoiseBiomeSource {
    parameters: ClimateIndex<String>,
}
impl MultiNoiseBiomeSource {
    pub fn new(entries: Vec<(ParameterPoint, String)>) -> Result<Self, BiomeError> {
        let entries = entries
            .into_iter()
            .map(|(point, name)| Ok((point, identifier(&name)?)))
            .collect::<Result<_, DecodeError>>()?;
        Ok(Self {
            parameters: ClimateIndex::new(entries)?,
        })
    }
    /// Horizon's quantized import format, not the vanilla biome definition
    /// codec: [{"coordinates":[[min,max]; six],"offset":long,"biome":id}].
    /// Built-in numeric tables stay private and are authenticated by the runner.
    pub fn from_quantized_json(json: &str) -> Result<Self, BiomeError> {
        let input: serde_json::Value = serde_json::from_str(json).map_err(DecodeError::from)?;
        let mut entries = Vec::new();
        for entry in input
            .as_array()
            .ok_or(DecodeError::Field("biome parameter list"))?
        {
            let raw = entry["coordinates"]
                .as_array()
                .filter(|v| v.len() == 6)
                .ok_or(DecodeError::Field("climate coordinates"))?;
            let mut coordinates = [Parameter::new(0, 0); 6];
            for (coordinate, raw) in coordinates.iter_mut().zip(raw) {
                let raw = raw
                    .as_array()
                    .filter(|v| v.len() == 2)
                    .ok_or(DecodeError::Field("climate interval"))?;
                *coordinate = Parameter::new(
                    raw[0]
                        .as_i64()
                        .ok_or(DecodeError::Field("climate minimum"))?,
                    raw[1]
                        .as_i64()
                        .ok_or(DecodeError::Field("climate maximum"))?,
                );
            }
            let offset = entry["offset"]
                .as_i64()
                .ok_or(DecodeError::Field("climate offset"))?;
            let biome = entry["biome"]
                .as_str()
                .ok_or(DecodeError::Field("biome identifier"))?;
            entries.push((
                ParameterPoint {
                    coordinates,
                    offset,
                },
                biome.to_owned(),
            ));
        }
        Self::new(entries)
    }
    pub fn parameters(&self) -> &ClimateIndex<String> {
        &self.parameters
    }
    pub fn resolver<'a>(&'a self, climate: &'a ClimateSampler) -> BiomeResolver<'a> {
        BiomeResolver {
            climate,
            search: self.parameters.searcher(),
        }
    }
}
#[derive(Debug)]
pub struct BiomeResolver<'a> {
    climate: &'a ClimateSampler,
    search: ClimateSearch<'a, String>,
}
impl<'a> BiomeResolver<'a> {
    pub fn sample(
        &mut self,
        context: &mut SamplerContext,
        quart: [i32; 3],
    ) -> Result<&'a str, BiomeError> {
        Ok(self
            .search
            .find(self.climate.sample(context, quart)?)?
            .as_str())
    }
    pub fn prepare_volume(
        &self,
        context: &mut SamplerContext,
        minimum: [i32; 3],
        extents: [i32; 3],
    ) -> Result<ClimateVolume, BiomeError> {
        Ok(self.climate.sample_volume(context, minimum, extents)?)
    }
    /// Query order shares the same previous-leaf state as scalar resolution.
    pub fn sample_prepared(
        &mut self,
        volume: &ClimateVolume,
        quart: [i32; 3],
    ) -> Result<&'a str, BiomeError> {
        let target = volume.target(quart).ok_or(BiomeError::OutsideVolume)?;
        Ok(self.search.find(target)?.as_str())
    }
}
