//! Prepared model choices with vanilla random draw consumption.
use super::{BlockStateDefinition, Variant, VariantSet, bind::SelectedModels};
use crate::json::{Result, error};
use horizon_core::random::RandomSource;

#[derive(Clone, Debug)]
enum Choice {
    Single(Variant),
    Weighted {
        variants: Vec<Variant>,
        ends: Vec<i32>,
        total: i32,
    },
}
impl Choice {
    fn new(value: &VariantSet) -> Result<Self> {
        match value {
            VariantSet::Single(value) => Ok(Self::Single(value.clone())),
            VariantSet::Weighted(entries) => {
                let mut total = 0i32;
                let mut ends = Vec::with_capacity(entries.len());
                let mut variants = Vec::with_capacity(entries.len());
                for (variant, weight) in entries {
                    if *weight <= 0 {
                        return Err(error("nonpositive model weight"));
                    }
                    total = total
                        .checked_add(*weight)
                        .ok_or_else(|| error("model weight overflow"))?;
                    ends.push(total);
                    variants.push(variant.clone());
                }
                if total == 0 {
                    return Err(error("empty weighted model"));
                }
                Ok(Self::Weighted {
                    variants,
                    ends,
                    total,
                })
            }
        }
    }
    fn select(&self, random: &mut impl RandomSource) -> &Variant {
        match self {
            Self::Single(value) => value,
            Self::Weighted {
                variants,
                ends,
                total,
            } => {
                // A singleton weighted list still consumes the bounded draw.
                let index = random
                    .next_int_bound(*total)
                    .expect("validated positive model weight");
                &variants[ends.partition_point(|end| *end <= index)]
            }
        }
    }
}
/// Compile once per definition and share across its bound block states. Prefix
/// sums avoid rebuilding weight totals or scanning all entries during selection.
#[derive(Clone, Debug)]
pub struct CompiledChoices {
    variants: Vec<Choice>,
    multipart: Vec<Choice>,
}
impl CompiledChoices {
    pub fn new(definition: &BlockStateDefinition) -> Result<Self> {
        Ok(Self {
            variants: definition
                .variants
                .iter()
                .flatten()
                .map(|(_, v)| Choice::new(v))
                .collect::<Result<_>>()?,
            multipart: definition
                .multipart
                .iter()
                .flatten()
                .map(|s| Choice::new(&s.variant))
                .collect::<Result<_>>()?,
        })
    }
    /// The variant `particleMaterial` comes from: the first entry of a weighted
    /// list (`WeightedVariants`) and, for a multipart root, of its first
    /// selector whether or not that selector applies (`MultiPartModel`).
    pub fn particle_variant(&self, selected: &SelectedModels) -> Option<&Variant> {
        let choice = match selected {
            SelectedModels::Variant(index) => self.variants.get(*index)?,
            SelectedModels::Multipart(_) => self.multipart.first()?,
        };
        Some(match choice {
            Choice::Single(value) => value,
            Choice::Weighted { variants, .. } => variants.first()?,
        })
    }
    /// Append selected variants without allocating intermediate part lists. A
    /// multipart root consumes nextLong even for an empty selected subset, then
    /// resets the same random source to that seed before each selected part.
    pub fn collect<'a>(
        &'a self,
        selected: &SelectedModels,
        random: &mut impl RandomSource,
        output: &mut Vec<&'a Variant>,
    ) -> Result<()> {
        match selected {
            SelectedModels::Variant(index) => {
                let choice = self
                    .variants
                    .get(*index)
                    .ok_or_else(|| error("invalid variant selection"))?;
                output.push(choice.select(random));
            }
            SelectedModels::Multipart(indices) => {
                if indices.iter().any(|index| *index >= self.multipart.len()) {
                    return Err(error("invalid multipart selection"));
                }
                let seed = random.next_long();
                for index in indices {
                    random.set_seed(seed);
                    output.push(self.multipart[*index].select(random));
                }
            }
        }
        Ok(())
    }
}
