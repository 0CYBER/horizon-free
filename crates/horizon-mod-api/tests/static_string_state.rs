pub use mod_api::wire::*;
pub use mod_api::*;

mod tests {
    use super::*;
    #[test]
    fn static_string_rows_roundtrip_through_intern() {
        mod_api::__private::initialize().unwrap();
        let rows: Vec<(&'static str, &'static str)> =
            vec![("test:block", "test:glow"), ("test:block", "test:shade")];
        let mut out = super::Writer::new();
        out.put(&rows);
        let decoded =
            super::Reader::decode::<Vec<(&'static str, &'static str)>>(out.bytes()).unwrap();
        assert_eq!(decoded, rows);
        assert!(std::ptr::eq(decoded[0].0.as_ptr(), decoded[1].0.as_ptr()));
        let mut oversized = super::Writer::new();
        oversized.put(&"x".repeat(257));
        assert_eq!(
            super::Reader::decode::<&'static str>(oversized.bytes()),
            Err(super::WireError::Invalid("static string too long"))
        );
    }

    #[test]
    fn attachment_refusals_use_bounded_interned_static_text() {
        mod_api::__private::initialize().unwrap();
        round(crate::AttachmentError::HolderBusy);
        round(crate::AttachmentError::UnsupportedHolder("dragonpart"));
        round(crate::AttachmentRowError::HolderBusy);
        round(crate::AttachmentRowError::UnsupportedHolder("dragonpart"));
        let bound: &'static str = Box::leak("é".repeat(128).into_boxed_str());
        round(crate::AttachmentError::UnsupportedHolder(bound));
        round(crate::AttachmentRowError::UnsupportedHolder(bound));
        let mut out = super::Writer::new();
        out.put(&crate::AttachmentError::UnsupportedHolder("dragon"));
        let first = super::Reader::decode::<crate::AttachmentError>(out.bytes()).unwrap();
        for _ in 0..32 {
            let next = super::Reader::decode::<crate::AttachmentError>(out.bytes()).unwrap();
            let (
                crate::AttachmentError::UnsupportedHolder(a),
                crate::AttachmentError::UnsupportedHolder(b),
            ) = (first, next)
            else {
                unreachable!()
            };
            assert!(std::ptr::eq(a.as_ptr(), b.as_ptr()));
        }
        out.clear();
        out.put(&8u32);
        out.put(&"x".repeat(257));
        assert_eq!(
            super::Reader::decode::<crate::AttachmentError>(out.bytes()),
            Err(super::WireError::Invalid("static string too long"))
        );
        out.clear();
        out.put(&3u32);
        out.put(&"é".repeat(129));
        assert_eq!(
            super::Reader::decode::<crate::AttachmentRowError>(out.bytes()),
            Err(super::WireError::Invalid("static string too long"))
        );
    }

    fn round<T: Wire + PartialEq + std::fmt::Debug>(value: T) {
        let mut out = Writer::new();
        out.put(&value);
        assert_eq!(Reader::decode::<T>(out.bytes()), Ok(value));
    }
}
