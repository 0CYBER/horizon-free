//! Lossless host directory values available during registration.

/// The game's instance directory, captured before registration begins.
///
/// This is a path value, not filesystem authority. Wasm receives the same data
/// as native mods, including the host's encoding; it gains no filesystem access.
/// Never interpret Unix bytes as UTF-8 or discard unpaired Windows surrogates.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum InstanceDirectory {
    /// A Unix path, preserving every byte, including non-UTF-8 names.
    Unix(Vec<u8>),
    /// A Windows path as UTF-16 code units, preserving unpaired surrogates.
    Windows(Vec<u16>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Reader, Writer};

    #[test]
    fn directory_wire_preserves_non_unicode_host_paths() {
        for directory in [
            InstanceDirectory::Unix(vec![b'/', 0xff, 0x80]),
            InstanceDirectory::Windows(vec![b'C' as u16, 58, 92, 0xd800, 0xffff]),
        ] {
            let mut output = Writer::new();
            output.put(&directory);
            let mut input = Reader::new(output.bytes());
            let decoded: InstanceDirectory = input.take().unwrap();
            assert_eq!(decoded, directory);
        }
    }
}
