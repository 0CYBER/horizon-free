/// A previously requested block in the game fixture. Unknown names are a fixture error.
pub fn block_key(id: &str) -> BlockKey {
    let canonical = if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    };
    mod_vocab::__private::parts_installation::requested_block_key(&canonical)
        .expect("fixture requested this block")
}
