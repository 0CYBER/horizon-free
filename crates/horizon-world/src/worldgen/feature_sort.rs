//! Deterministic feature ordering across biome generation steps.
use std::collections::{BTreeMap, BTreeSet};

/// A placed-feature object identity, assigned by the owning registry snapshot.
/// Structurally equal but separately allocated features need distinct IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeatureIdentity(pub u32);

#[derive(Clone, Debug)]
pub struct StepFeatureData {
    features: Vec<FeatureIdentity>,
    indices: BTreeMap<FeatureIdentity, usize>,
}
impl StepFeatureData {
    pub fn features(&self) -> &[FeatureIdentity] {
        &self.features
    }
    pub fn index(&self, feature: FeatureIdentity) -> Option<usize> {
        self.indices.get(&feature).copied()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureSortError {
    Cycle,
    WorkLimit,
    DuplicateBiome,
}
impl std::fmt::Display for FeatureSortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cycle => "feature order cycle found",
            Self::WorkLimit => "feature ordering input limit exceeded",
            Self::DuplicateBiome => "duplicate biome identity in decoration plan",
        })
    }
}
impl std::error::Error for FeatureSortError {}

/// Inputs preserve biome order, step order and each holder set's encounter order.
pub fn build_features_per_step(
    biomes: &[Vec<Vec<FeatureIdentity>>],
) -> Result<Vec<StepFeatureData>, FeatureSortError> {
    if biomes.len() > 8192 {
        return Err(FeatureSortError::WorkLimit);
    }
    type Node = (usize, usize); // Native comparator: step, then first identity index.
    let mut identities = BTreeMap::new();
    let mut encountered = Vec::new();
    let mut graph = BTreeMap::<Node, BTreeSet<Node>>::new();
    let mut steps = 0;
    let mut remaining = 65536usize;
    for biome in biomes {
        if biome.len() > 8192 {
            return Err(FeatureSortError::WorkLimit);
        }
        steps = steps.max(biome.len());
        let mut previous = None;
        for (step, features) in biome.iter().enumerate() {
            remaining = remaining
                .checked_sub(1)
                .ok_or(FeatureSortError::WorkLimit)?;
            for feature in features {
                remaining = remaining
                    .checked_sub(1)
                    .ok_or(FeatureSortError::WorkLimit)?;
                let index = *identities.entry(*feature).or_insert_with(|| {
                    encountered.push(*feature);
                    encountered.len() - 1
                });
                let node = (step, index);
                graph.entry(node).or_default();
                if let Some(previous) = previous {
                    graph.entry(previous).or_default().insert(node);
                }
                previous = Some(node);
            }
        }
    }
    let mut colors = BTreeMap::<Node, u8>::new();
    let mut ordered = Vec::with_capacity(graph.len());
    let mut stack = Vec::new();
    for &root in graph.keys() {
        if colors.get(&root) == Some(&2) {
            continue;
        }
        stack.push((root, false));
        while let Some((node, finish)) = stack.pop() {
            if finish {
                colors.insert(node, 2);
                ordered.push(node);
                continue;
            }
            match colors.get(&node) {
                Some(2) => continue,
                Some(1) => return Err(FeatureSortError::Cycle),
                _ => {}
            }
            colors.insert(node, 1);
            stack.push((node, true));
            stack.extend(graph[&node].iter().rev().map(|&next| (next, false)));
        }
    }
    let mut result = (0..steps)
        .map(|_| StepFeatureData {
            features: Vec::new(),
            indices: BTreeMap::new(),
        })
        .collect::<Vec<_>>();
    for (step, index) in ordered.into_iter().rev() {
        let feature = encountered[index];
        let data = &mut result[step];
        data.indices.insert(feature, data.features.len());
        data.features.push(feature);
    }
    Ok(result)
}
