//! Public payload callback results use the canonical Wire codec on both targets.
use mod_api::wire::{Reader, WireError, Writer};
use mod_api::{PayloadBusy, PayloadBytes, PayloadHandleError};
use std::sync::Arc;

fn holder() -> PayloadBusy {
    PayloadBusy {
        owner: Arc::from("example"),
        export: Arc::from("configuration"),
        thread: Arc::from("payload-network"),
        held_millis: 31_337,
    }
}

#[test]
fn callback_success_and_real_failure_have_distinct_wire_results() {
    let mut out = Writer::new();
    out.put(&Ok::<(), PayloadHandleError>(()));
    assert_eq!(out.error(), None);
    assert_eq!(out.bytes(), &[0]);
    assert_eq!(
        Reader::decode::<Result<(), PayloadHandleError>>(out.bytes()),
        Ok(Ok(()))
    );

    out.clear();
    let result = Err::<(), _>(PayloadHandleError::Failed("bad".into()));
    out.put(&result);
    assert_eq!(out.error(), None);
    assert_eq!(out.bytes(), &[1, 0, 0, 0, 0, 3, 0, 0, 0, b'b', b'a', b'd']);
    assert_eq!(
        Reader::decode::<Result<(), PayloadHandleError>>(out.bytes()),
        Ok(result)
    );
}

#[test]
fn busy_result_encodes_exact_application_bytes_and_resolved_holder_fields() {
    let result = Err::<(), _>(PayloadHandleError::Busy {
        payload: PayloadBytes(vec![0, 255, 13, 7]),
        holder: holder(),
    });
    let mut borrowed = Writer::new();
    borrowed.put(&result);
    assert_eq!(borrowed.error(), None);

    // Decode through independent primitive field types, not the derived result itself.
    let mut fields = Reader::new(borrowed.bytes());
    assert_eq!(fields.take::<u8>(), Ok(1));
    assert_eq!(fields.take::<u32>(), Ok(1));
    assert_eq!(fields.take::<Vec<u8>>(), Ok(vec![0, 255, 13, 7]));
    assert_eq!(fields.take::<String>(), Ok("example".into()));
    assert_eq!(fields.take::<String>(), Ok("configuration".into()));
    assert_eq!(fields.take::<String>(), Ok("payload-network".into()));
    assert_eq!(fields.take::<u64>(), Ok(31_337));
    assert!(fields.rest().is_empty());

    let decoded = Reader::decode::<Result<(), PayloadHandleError>>(borrowed.bytes()).unwrap();
    assert_eq!(decoded, result);
    let mut owned = Writer::new();
    owned.put_owned(result);
    assert_eq!(owned.error(), None);
    assert_eq!(owned.bytes(), borrowed.bytes());
}

#[test]
fn invalid_callback_results_refuse_tags_truncation_and_trailing_data() {
    assert_eq!(
        Reader::decode::<Result<(), PayloadHandleError>>(&[2]),
        Err(WireError::Tag(2))
    );
    assert_eq!(
        Reader::decode::<Result<(), PayloadHandleError>>(&[1, 2, 0, 0, 0]),
        Err(WireError::Tag(2))
    );
    let mut out = Writer::new();
    out.put(&Err::<(), _>(PayloadHandleError::Busy {
        payload: PayloadBytes(vec![9, 4, 2]),
        holder: holder(),
    }));
    assert_eq!(out.error(), None);
    for end in 0..out.bytes().len() {
        assert!(
            Reader::decode::<Result<(), PayloadHandleError>>(&out.bytes()[..end]).is_err(),
            "accepted a truncated result at {end}"
        );
    }
    let mut trailing = out.bytes().to_vec();
    trailing.push(0);
    assert_eq!(
        Reader::decode::<Result<(), PayloadHandleError>>(&trailing),
        Err(WireError::Trailing)
    );
}

#[test]
fn holder_snapshots_share_resolved_names_and_format_all_diagnostic_fields() {
    let original = holder();
    let snapshot = original.clone();
    assert!(Arc::ptr_eq(&original.owner, &snapshot.owner));
    assert!(Arc::ptr_eq(&original.export, &snapshot.export));
    assert!(Arc::ptr_eq(&original.thread, &snapshot.thread));
    assert_eq!(snapshot.held_millis, 31_337);
    assert_eq!(
        snapshot.to_string(),
        "mod example export configuration on thread payload-network held for 31337 ms"
    );
}
