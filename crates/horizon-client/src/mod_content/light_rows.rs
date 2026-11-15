//! The light a declared block's gameplay row takes: `properties.light`, its
//! `state_light` over it, and `dynamic_light`.
use super::{J, NewBlock};

/// The declared light of `b`'s state `state_index`, on its gameplay row.
pub(super) fn override_light(b: &NewBlock<'_>, state_index: usize, row: &mut J) {
    let p = &b.decl.properties;
    if let Some(v) = p.light {
        row.set("emission", J::Num(v.into()));
    }
    if let Some(values) = &b.state_light {
        row.set("emission", J::Num(values[state_index].into()));
    }
    if p.dynamic_light == Some(true) {
        row.set("dynamic_light", J::Bool(true));
    }
}
