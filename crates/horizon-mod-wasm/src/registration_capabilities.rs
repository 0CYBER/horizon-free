//! Registration hooks whose borrowed context cannot yet cross the Wire boundary.
use crate::Refusal;

pub(crate) fn check(subscriptions: &[(String, i32, bool)]) -> Result<(), Refusal> {
    if subscriptions
        .iter()
        .any(|(name, _, _)| name == "extend_poi_types")
    {
        return Err(Refusal::Runtime(
            "extend_poi_types requires PoiTypeExtensions::metadata, whose borrowed BlockMetadata accessor cannot cross the wasm Wire boundary".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_before_registration_with_a_named_capability_reason() {
        let error = check(&[("extend_poi_types".into(), 0, false)]).unwrap_err();
        let reason = error.to_string();
        assert!(reason.contains("PoiTypeExtensions::metadata"), "{reason}");
        assert!(reason.contains("wasm Wire boundary"), "{reason}");
        assert!(check(&[("server_started".into(), 0, false)]).is_ok());
    }
}
