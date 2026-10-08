//! Bind resource selectors to a block's property schema and ordered states.
use super::{BlockStateDefinition, Condition, Term};
use crate::json::{Result, error};
use horizon_core::text::parse_java_integer;
use std::collections::{BTreeMap, BTreeSet};

pub type Properties = BTreeMap<String, String>;
pub type PropertySchema = BTreeMap<String, PropertyValues>;
#[derive(Clone, Debug)]
pub enum PropertyValues {
    Named(BTreeSet<String>),
    Integer { minimum: i32, maximum: i32 },
}
impl PropertyValues {
    fn parse(&self, text: &str) -> Option<String> {
        match self {
            Self::Named(values) => values.contains(text).then(|| text.to_owned()),
            Self::Integer { minimum, maximum } => {
                let value = parse_java_integer(text)?;
                (value >= *minimum && value <= *maximum).then(|| value.to_string())
            }
        }
    }
}
/// Indices into the original definition. An empty multipart selection is distinct
/// from an absent root: the native root exists but contributes no model parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectedModels {
    Variant(usize),
    Multipart(Vec<usize>),
}
#[derive(Clone, Debug)]
enum BoundCondition {
    Or(Vec<Self>),
    And(Vec<Self>),
    Properties(BTreeMap<String, Vec<Term>>),
}
impl BoundCondition {
    fn bind(value: &Condition, schema: &PropertySchema) -> Result<Self> {
        Ok(match value {
            Condition::Or(values) => Self::Or(
                values
                    .iter()
                    .map(|v| Self::bind(v, schema))
                    .collect::<Result<_>>()?,
            ),
            Condition::And(values) => Self::And(
                values
                    .iter()
                    .map(|v| Self::bind(v, schema))
                    .collect::<Result<_>>()?,
            ),
            Condition::Properties(values) => Self::Properties(
                values
                    .iter()
                    .map(|(key, terms)| {
                        let domain = schema
                            .get(key)
                            .ok_or_else(|| error("unknown multipart property"))?;
                        let terms = terms
                            .iter()
                            .map(|t| {
                                Ok(Term {
                                    value: domain
                                        .parse(&t.value)
                                        .ok_or_else(|| error("unknown multipart property value"))?,
                                    negated: t.negated,
                                })
                            })
                            .collect::<Result<_>>()?;
                        Ok((key.clone(), terms))
                    })
                    .collect::<Result<_>>()?,
            ),
        })
    }
    fn matches(&self, state: &Properties) -> bool {
        match self {
            Self::Or(values) => values.iter().any(|v| v.matches(state)),
            Self::And(values) => values.iter().all(|v| v.matches(state)),
            Self::Properties(values) => values.iter().all(|(key, terms)| {
                state
                    .get(key)
                    .is_some_and(|value| terms.iter().any(|t| (value == &t.value) ^ t.negated))
            }),
        }
    }
}
fn variant_predicate(text: &str, schema: &PropertySchema) -> Result<Properties> {
    let mut result = Properties::new();
    for clause in text.split(',') {
        let (key, value) = clause
            .split_once('=')
            .map_or((clause, None), |(k, v)| (k, Some(v)));
        if let (Some(domain), Some(value)) = (schema.get(key), value) {
            result.insert(
                key.to_owned(),
                domain
                    .parse(value)
                    .ok_or_else(|| error("unknown variant property value"))?,
            );
        } else if !key.is_empty() {
            return Err(error("unknown or incomplete variant property"));
        }
    }
    Ok(result)
}
impl BlockStateDefinition {
    /// `states` must be the block's canonical possible states in native order.
    /// Invalid variant selectors are skipped. On overlapping selectors, vanilla
    /// replaces the first overlapping state before aborting that selector's walk.
    /// Multipart predicates bind eagerly, even when variants cover every state.
    pub fn bind(
        &self,
        schema: &PropertySchema,
        states: &[Properties],
    ) -> Result<Vec<Option<SelectedModels>>> {
        let mut result = vec![None; states.len()];
        if let Some(variants) = &self.variants {
            for (index, (key, _)) in variants.iter().enumerate() {
                let Ok(predicate) = variant_predicate(key, schema) else {
                    continue;
                };
                for (target, state) in result.iter_mut().zip(states) {
                    if predicate
                        .iter()
                        .all(|(key, value)| state.get(key) == Some(value))
                        && target.replace(SelectedModels::Variant(index)).is_some()
                    {
                        break;
                    }
                }
            }
        }
        if let Some(selectors) = &self.multipart {
            let predicates = selectors
                .iter()
                .map(|s| {
                    s.condition
                        .as_ref()
                        .map(|v| BoundCondition::bind(v, schema))
                        .transpose()
                })
                .collect::<Result<Vec<_>>>()?;
            for (target, state) in result.iter_mut().zip(states) {
                if target.is_none() {
                    let selected = predicates
                        .iter()
                        .enumerate()
                        .filter_map(|(index, p)| {
                            p.as_ref().is_none_or(|p| p.matches(state)).then_some(index)
                        })
                        .collect();
                    *target = Some(SelectedModels::Multipart(selected));
                }
            }
        }
        Ok(result)
    }
}
