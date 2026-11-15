//! Typed packets per connection state and direction. Every state has a
//! `Clientbound` and a `Serverbound` enum whose `decode` checks that the
//! whole body was consumed (vanilla rejects oversized packets the same way)
//! and whose `encode` writes the id and payload. Packets Horizon does not
//! model decode as `Unknown` and keep their raw payload.
pub mod configuration;
pub mod custom_payload;
pub mod handshake;
pub mod login;
pub mod play;
pub mod recipes;
pub mod status;

use crate::buf::{Codec, Reader, Writer};

/// Generate a state's packet enum from (Variant(Type) = ID) entries.
#[macro_export]
macro_rules! packet_enum {
    ($(#[$meta:meta])* $name:ident, $ids:path { $($variant:ident($ty:ty) = $id:ident $(=> $codec:ty)?),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq)]
        pub enum $name {
            $($variant($ty),)*
            /// A packet Horizon does not model: id and raw payload.
            Unknown(i32, Vec<u8>),
        }
        impl $crate::retained::OwnedHeap for $name {
            fn owned_heap_bytes(&self) -> usize { match self {
                $(Self::$variant(value) => $crate::retained::OwnedHeap::owned_heap_bytes(value),)*
                Self::Unknown(_, bytes) => bytes.capacity(),
            } }
        }
        impl $name {
            /// Retained heap charge; opaque map storage uses conservative std-layout bounds.
            pub fn owned_heap_bytes(&self) -> usize { $crate::retained::OwnedHeap::owned_heap_bytes(self) }

            pub fn id(&self) -> i32 {
                use $ids as ids;
                match self {
                    $(Self::$variant(_) => ids::$id,)*
                    Self::Unknown(id, _) => *id,
                }
            }
            /// The packet's name in this enum (`MoveVehicle`), for logs.
            pub fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant(_) => stringify!($variant),)*
                    Self::Unknown(..) => "Unknown",
                }
            }
            /// Decode a packet body (after the id).
            pub fn decode(id: i32, payload: &[u8]) -> $crate::Result<Self> {
                Self::decode_reader(id, $crate::buf::Reader::new(payload))
            }
            /// Decode with the local receiving codecs frozen at mod load.
            pub fn decode_with_payload_codecs(id: i32, payload: &[u8], codecs: &$crate::packets::custom_payload::PayloadCodecs) -> $crate::Result<Self> {
                Self::decode_reader(id, $crate::buf::Reader::with_payload_codecs(payload, codecs))
            }
            fn decode_reader(id: i32, mut r: $crate::buf::Reader<'_>) -> $crate::Result<Self> {
                use $ids as ids;
                let packet = match id {
                    $(ids::$id => Self::$variant(<$crate::packet_codec!($ty $(, $codec)?) as $crate::buf::Codec<$ty>>::read(&mut r)?),)*
                    _ => return Ok(Self::Unknown(id, r.rest().to_vec())),
                };
                if r.remaining() != 0 {
                    return Err($crate::Error::new(format!(
                        "packet {} ({}) was larger than expected: {} extra bytes",
                        id,
                        stringify!($name),
                        r.remaining()
                    )));
                }
                Ok(packet)
            }
            /// Validate directional constraints before allocating an encoded body.
            pub fn try_encode(&self) -> $crate::Result<Vec<u8>> {
                match self {
                    $(Self::$variant(p) => <$crate::packet_codec!($ty $(, $codec)?) as $crate::buf::Codec<$ty>>::validate(p)?,)*
                    Self::Unknown(_, _) => {}
                }
                Ok(self.encode())
            }
            /// Validate registered custom payloads in this connection's codec context.
            pub fn try_encode_with_payload_codecs(&self, codecs: &$crate::packets::custom_payload::PayloadCodecs) -> $crate::Result<Vec<u8>> {
                match self {
                    $(Self::$variant(p) => <$crate::packet_codec!($ty $(, $codec)?) as $crate::buf::Codec<$ty>>::validate_with_payload_codecs(p, codecs)?,)*
                    Self::Unknown(_, _) => {}
                }
                Ok(self.encode())
            }
            /// The packet body: VarInt id then payload, for trusted values.
            /// Use `try_encode` for values supplied by mods or other untrusted producers.
            pub fn encode(&self) -> Vec<u8> {
                let mut w = $crate::buf::Writer::new();
                w.varint(self.id());
                match self {
                    $(Self::$variant(p) => <$crate::packet_codec!($ty $(, $codec)?) as $crate::buf::Codec<$ty>>::write(p, &mut w),)*
                    Self::Unknown(_, payload) => w.bytes(payload),
                }
                w.data
            }
        }
    };
}

/// Select the default packet codec or an explicitly directional codec.
#[doc(hidden)]
#[macro_export]
macro_rules! packet_codec {
    ($ty:ty) => {
        $ty
    };
    ($ty:ty, $codec:ty) => {
        $codec
    };
}

/// A packet with no fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Empty;
impl Codec<Empty> for Empty {
    fn read(_: &mut Reader) -> crate::Result<Empty> {
        Ok(Empty)
    }
    fn write(_: &Empty, _: &mut Writer) {}
}

// Generated retained-allocation traversals; regenerate with protocol-retained-gen.
impl crate::retained::OwnedHeap for Empty {
    fn owned_heap_bytes(&self) -> usize {
        let Self = self;
        0usize
    }
}
// End generated retained traversals.

#[cfg(test)]
pub(crate) fn round_trip<T: Clone + PartialEq + std::fmt::Debug>(
    packet: T,
    encode: impl Fn(&T) -> Vec<u8>,
    decode: impl Fn(i32, &[u8]) -> crate::Result<T>,
) {
    let body = encode(&packet);
    let mut r = Reader::new(&body);
    let id = r.varint().unwrap();
    let back = decode(id, r.rest()).unwrap();
    assert_eq!(back, packet);
}
