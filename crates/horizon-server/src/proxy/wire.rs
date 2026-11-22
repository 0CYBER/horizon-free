//! The protocol pieces the proxy speaks, through the platform's public API (docs/layers.md).

pub use horizon_platform::protocol::{
    Error, PROTOCOL_VERSION,
    buf::{Reader, Writer, write_varint},
    connection::{Closer, ConnReader, ConnWriter, Connection},
    crypto::{offline_uuid, random_bytes, server_hash},
    ids,
    packets::{Empty, handshake, login, status},
    rsa::RsaPrivateKey,
};

/// A translatable component as JSON, for the login and the server list: the client shows it in its own
/// language, and `fallback` where it has no such key (a vanilla client).
pub fn translated_json(key: &str, fallback: &str) -> String {
    serde_json::json!({ "translate": key, "fallback": fallback }).to_string()
}

pub use horizon_platform::world::nbt::{JavaString, Tag};

/// A translatable component the client shows in its own language, `fallback` where it has no such key (a
/// vanilla client, or a key from after its release): as NBT for configuration and play.
pub fn translated(key: &str, fallback: &str) -> Tag {
    Tag::compound(vec![
        ("translate", Tag::String(JavaString::from(key))),
        ("fallback", Tag::String(JavaString::from(fallback))),
    ])
}

/// A text component written as JSON (what a login's disconnect carries), as the NBT configuration and play
/// take: a string stays a string, an object a compound, an array a list (each item a compound, as NBT lists
/// are of one type).
pub fn component_from_json(json: &str) -> Tag {
    fn convert(v: &serde_json::Value, in_list: bool) -> Tag {
        match v {
            serde_json::Value::String(s) if in_list => {
                Tag::compound(vec![("text", Tag::String(JavaString::from(s.as_str())))])
            }
            serde_json::Value::String(s) => Tag::String(JavaString::from(s.as_str())),
            serde_json::Value::Bool(b) => Tag::Byte(i8::from(*b)),
            serde_json::Value::Number(n) => match n.as_i64().and_then(|i| i32::try_from(i).ok()) {
                Some(i) => Tag::Int(i),
                None => Tag::String(JavaString::from(n.to_string().as_str())),
            },
            serde_json::Value::Array(items) => {
                Tag::List(items.iter().map(|i| convert(i, true)).collect())
            }
            serde_json::Value::Object(map) => Tag::compound(
                map.iter()
                    .map(|(k, v)| (k.as_str(), convert(v, false)))
                    .collect::<Vec<_>>(),
            ),
            serde_json::Value::Null => Tag::String(JavaString::from("")),
        }
    }
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(v) => convert(&v, false),
        Err(_) => Tag::String(JavaString::from(json)),
    }
}

/// The game version a server list shows.
pub fn version_name() -> &'static str {
    horizon_platform::version::VANILLA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_refusal_becomes_the_component_play_and_configuration_take() {
        let tag = component_from_json(
            r#"{"translate":"multiplayer.disconnect.banned.reason","with":["griefing"]}"#,
        );
        let Tag::Compound(map) = &tag else {
            panic!("{tag:?}")
        };
        assert_eq!(
            map.get(&JavaString::from("translate")),
            Some(&Tag::String(JavaString::from(
                "multiplayer.disconnect.banned.reason"
            )))
        );
        // `with`'s items are components too: a string in a list becomes a text compound.
        let Some(Tag::List(with)) = map.get(&JavaString::from("with")) else {
            panic!("{tag:?}")
        };
        assert!(matches!(&with[0], Tag::Compound(m) if m.get(&JavaString::from("text")).is_some()));
        assert_eq!(
            component_from_json("\"plain\""),
            Tag::String(JavaString::from("plain"))
        );
    }
}
