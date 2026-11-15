//! Compile and behavioral proof for actual point-shape generated records.
#![cfg(feature = "wasm-guest")]
include!(concat!(env!("OUT_DIR"), "/records.rs"));

#[cfg(test)]
mod tests {
    use super::{changed, original};

    /// The mod's hooks, which `wasm-guest` declares and `#[mod_api::horizon_mod]` defines in a real mod. The
    /// native test binary needs a definition too: MSVC's linker refuses the unresolved symbol even though nothing
    /// here calls it, where GNU ld drops the unused reference.
    #[unsafe(no_mangle)]
    fn __horizon_mod_hooks() -> Box<dyn mod_api::Hooks> {
        struct Empty;
        impl mod_api::Hooks for Empty {
            fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
                &[]
            }
        }
        Box::new(Empty)
    }
    use mod_api::wire::{Reader, Wire, WireError, Writer};
    use std::borrow::Cow;

    fn encoded() -> Vec<u8> {
        let mut record = original::ProofArgs::__new(
            Cow::Borrowed("hi"),
            7,
            original::Nested {
                child: original::Child { value: true },
            },
        );
        record.result = Some(false);
        let mut writer = Writer::new();
        record.put(&mut writer);
        assert_eq!(writer.error(), None);
        writer.into_bytes()
    }

    #[test]
    fn emitted_borrowing_record_round_trips_with_real_wire() {
        let bytes = encoded();
        // Independent wire expectation: string length + UTF-8, u32 scalar,
        // nested bool, Option tag, bool outcome. No copied record declaration.
        assert_eq!(bytes, [2, 0, 0, 0, b'h', b'i', 7, 0, 0, 0, 1, 1, 0]);
        let decoded: original::ProofArgs<'_> = Reader::decode(&bytes).unwrap();
        assert_eq!(decoded.text, "hi");
        assert_eq!(decoded.count, 7);
        assert!(decoded.nested.child.value);
        assert_eq!(decoded.result, Some(false));
        // A listener may replace the borrowed input with owned text.
        let mut decoded = decoded;
        decoded.text = Cow::Owned("replacement".into());
        decoded.result = None;
        let mut writer = Writer::new();
        decoded.put(&mut writer);
        let again: original::ProofArgs<'_> = Reader::decode(writer.bytes()).unwrap();
        assert_eq!(again.text, "replacement");
        assert_eq!(again.result, None);
    }

    #[test]
    fn emitted_record_rejects_truncation_invalid_nested_and_outcome_values() {
        let bytes = encoded();
        for end in 0..bytes.len() {
            assert!(
                Reader::decode::<original::ProofArgs<'_>>(&bytes[..end]).is_err(),
                "prefix {end}"
            );
        }
        for offset in [10, 11, 12] {
            let mut invalid = bytes.clone();
            invalid[offset] = 2;
            assert!(
                Reader::decode::<original::ProofArgs<'_>>(&invalid).is_err(),
                "invalid byte at {offset}"
            );
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(matches!(
            Reader::decode::<original::ProofArgs<'_>>(&trailing),
            Err(WireError::Trailing)
        ));
    }

    #[test]
    fn emitted_layout_tracks_transitive_field_encoding() {
        // Names, record signature, field order and outcome stay identical;
        // only Child.value changes bool -> u32 in the build script.
        assert_eq!(
            original::ProofArgs::LAYOUT_SEED,
            changed::ProofArgs::LAYOUT_SEED
        );
        assert_ne!(
            original::Child::layout(&mut vec![]),
            changed::Child::layout(&mut vec![])
        );
        assert_ne!(
            original::Nested::layout(&mut vec![]),
            changed::Nested::layout(&mut vec![])
        );
        assert_ne!(
            original::ProofArgs::layout(&mut vec![]),
            changed::ProofArgs::layout(&mut vec![])
        );
    }
}
