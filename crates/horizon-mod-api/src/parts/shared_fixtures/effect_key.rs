/// A registered effect exposed by the real load-owned mapping, also for unrequested vanilla entries.
pub fn effect_key(id: &str) -> EffectKey {
    let canonical = if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    };
    mod_vocab::__private::parts_installation::resolved_effect_key(&canonical)
        .expect("fixture registered this effect")
}
