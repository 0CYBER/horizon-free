//! Cache admission failure must not change the static-string API's result.
//! Separate process: exhausting the shared native budget must not race other fixtures.
// Generated fixtures exercise every shape; TODO(arismoko/horizon#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
#[path = "support/no_hooks.rs"]
mod no_hooks;
use mod_api::wire::{Writer, guest};

#[test]
fn full_group_budget_leaves_static_replies_usable_without_acknowledging_unknown_ids() {
    mod_api::__private::initialize().unwrap();
    fn decode(tag: u8, text: Option<&str>) -> Result<&'static str, mod_api::wire::WireError> {
        let mut encoded = Writer::new();
        encoded.put(&tag);
        encoded.put(&0u32);
        if let Some(text) = text {
            encoded.put(text);
        }
        let encoded = encoded.into_bytes();
        guest::set_transport(Some(std::rc::Rc::new(move |_, _| encoded.clone())));
        let result = guest::host(0, guest::begin())
            .take_static::<()>()
            .map(|(value, ())| value);
        guest::set_transport(None);
        result
    }
    guest::reset_static_strings();
    assert!(guest::group_budget_try_reserve(guest::GROUP_BUDGET_BYTES));
    let uncached = decode(1, Some("test:budget_pressure")).unwrap();
    assert_eq!(uncached, "test:budget_pressure");
    assert_eq!(guest::static_ack(), u32::MAX);
    assert!(decode(0, None).is_err());
    guest::group_budget_release(guest::GROUP_BUDGET_BYTES);
    let admitted = decode(1, Some("test:budget_pressure")).unwrap();
    assert!(
        std::ptr::eq(uncached, admitted),
        "admission preserves interned identity"
    );
    assert_eq!(guest::static_ack(), 0);
    assert!(std::ptr::eq(admitted, decode(0, None).unwrap()));
    guest::reset_static_strings();
    assert!(
        guest::group_budget_try_reserve(guest::GROUP_BUDGET_BYTES),
        "reset releases vector reservations"
    );
    guest::group_budget_release(guest::GROUP_BUDGET_BYTES);
}
