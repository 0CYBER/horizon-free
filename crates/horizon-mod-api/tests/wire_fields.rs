use mod_api::wire::{Fields, ReadOnly, Record, Writer};
use std::borrow::Cow;

#[test]
fn readonly_fields_are_decoded_but_never_written_back() {
    let original = 7u32;
    let mut changed = false;
    let mut fields = Fields((ReadOnly(&original), (&mut changed, ())));
    let mut reply = Writer::new();
    reply.put(&99u32);
    reply.put(&true);
    fields.apply(reply.bytes()).unwrap();
    assert_eq!(original, 7);
    assert!(changed);
}

#[test]
fn fields_share_record_encoding_and_apply_atomically() {
    let (mut count, mut text, mut cancel) = (7u32, Cow::Borrowed("before"), false);
    let mut fields = Fields((&mut count, (&mut text, (&mut cancel, ()))));
    let mut expected = Writer::new();
    expected.put(&7u32);
    expected.put("before");
    expected.put(&false);
    let mut actual = Writer::new();
    fields.encode(&mut actual);
    assert_eq!(actual.bytes(), expected.bytes());

    let mut answer = Writer::new();
    answer.put(&11u32);
    answer.put("after");
    answer.put(&true);
    let bytes = answer.bytes();
    for end in 0..bytes.len() {
        assert!(fields.apply(&bytes[..end]).is_err());
        let mut unchanged = Writer::new();
        fields.encode(&mut unchanged);
        assert_eq!(unchanged.bytes(), expected.bytes());
    }
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(fields.apply(&trailing).is_err());
    fields.apply(bytes).unwrap();
    assert_eq!((count, text.as_ref(), cancel), (11, "after", true));
}
