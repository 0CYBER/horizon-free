/// An id with its namespace: `stone` is `minecraft:stone`.
pub(super) fn namespaced(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}
