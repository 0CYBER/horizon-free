#![cfg(test)]
#[test]
fn client_only_attribute_does_not_shift_both_side_registry() {
    use crate::Sentiment;
    use crate::parts::{AttributesRegistrar, PartRegistrar};
    let packages = [("alpha", crate::Side::Client), ("zeta", crate::Side::Both)]
        .into_iter()
        .map(|(id, side)| {
            PackageInfo::new(
                id.to_owned(),
                "1".to_owned(),
                PackageKind::Code,
                side,
                vec![id.to_owned()],
                vec![],
            )
        })
        .collect::<Vec<_>>();
    crate::load_state::with_test_package_inventory(packages, || {
        let mut alpha = PartRegistrar::new("alpha");
        let mut zeta = PartRegistrar::new("zeta");
        assert_eq!(
            alpha.declare_attribute(
                "decoration",
                0.0,
                0.0,
                1.0,
                false,
                Sentiment::Neutral,
                Default::default()
            ),
            Err(crate::AttributeRegistrationError::WrongSide)
        );
        assert!(
            zeta.declare_attribute(
                "mana",
                0.0,
                0.0,
                1.0,
                true,
                Sentiment::Positive,
                Default::default()
            )
            .is_ok()
        );
        assert_eq!(
            alpha.widen_attribute_bounds("minecraft:max_health", 1.0, 1e6),
            Err(crate::AttributeRegistrationError::WrongSide)
        );
        assert!(
            zeta.widen_attribute_bounds("minecraft:max_health", 1.0, 1e6)
                .is_ok()
        );
        assert!(alpha.attributes.is_empty() && alpha.attribute_widenings.is_empty());
        assert_eq!(zeta.attributes[0].id, "zeta:mana");
    });
}

#[cfg(test)]
use super::*;
