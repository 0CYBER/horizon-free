//! Concrete authoring of entity selector tokens: the checks `register_entity_selector` refuses with.
use super::*;

/// Vanilla's selector letters, which a mod's token never replaces.
const VANILLA: [&str; 6] = ["p", "a", "r", "s", "e", "n"];

impl EntitySelectorsRegistrar for PartRegistrar {
    fn add_entity_selector(&mut self, name: &str, selector: Arc<dyn crate::EntitySelectorType>) {
        let refusal = if name.is_empty()
            || name.len() > 32767
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            Some(format!(
                "{} registered invalid selector token @{name}",
                self.mod_id
            ))
        } else if VANILLA.contains(&name) {
            Some(format!(
                "{} registered vanilla's selector @{name}",
                self.mod_id
            ))
        } else if self
            .parts
            .entity_selectors
            .iter()
            .any(|other| *other.name == *name)
        {
            Some(format!(
                "{} registered selector token @{name} twice",
                self.mod_id
            ))
        } else {
            None
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        let base = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| selector.base()))
        {
            Ok(base) if base.limit != Some(0) && base.tooltip.encode_utf16().count() <= 32767 => {
                base
            }
            Ok(_) => {
                self.refused.push(format!(
                    "{} registered selector @{name} with zero limit or oversized tooltip",
                    self.mod_id
                ));
                return;
            }
            Err(_) => {
                self.refused.push(format!(
                    "{} registered selector @{name} whose base panicked",
                    self.mod_id
                ));
                return;
            }
        };
        // Horizon's bound, below the protocol payload cap. Size the exact revision-1 wire form.
        let varint_bytes =
            |n: usize| ((usize::BITS - n.leading_zeros()).max(1) as usize).div_ceil(7);
        let entry_bytes = varint_bytes(name.len())
            + name.len()
            + varint_bytes(base.tooltip.len())
            + base.tooltip.len()
            + 3
            + if base.limit.is_some() { 4 } else { 0 };
        let total = 1
            + varint_bytes(self.parts.entity_selectors.len() + 1)
            + self.entity_selector_bytes
            + entry_bytes;
        if total > 256 * 1024 {
            self.refused.push(format!("{} registered selector token @{name}: encoded metadata would exceed Horizon's 256 KiB total limit", self.mod_id));
            return;
        }
        self.entity_selector_bytes += entry_bytes;
        let index = self
            .parts
            .entity_selectors
            .partition_point(|info| info.name.as_ref() < name);
        self.parts.entity_selectors.insert(
            index,
            EntitySelectorInfo {
                name: name.into(),
                selector,
                base,
            },
        );
    }
}

/// Fixtures that tests of other crates use to install entity selector tokens.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use mod_vocab::EntitySelectorsRegistrar;

    #[test]
    fn aggregate_metadata_budget_refuses_only_the_next_token_without_mutating_prior_records() {
        struct Large;
        impl crate::EntitySelectorType for Large {
            fn base(&self) -> crate::SelectorBase {
                crate::SelectorBase::new(
                    false,
                    Some(1),
                    crate::SelectorSort::Nearest,
                    "x".repeat(32767),
                )
            }
            fn matches(
                &self,
                _: &mut dyn crate::ServerHost,
                _: &crate::CommandSource,
                _: crate::EntityRef,
            ) -> bool {
                true
            }
        }
        let mut registrar = super::PartRegistrar::new("budget");
        for n in 0..8 {
            registrar.add_entity_selector(&format!("t{n}"), std::sync::Arc::new(Large));
        }
        assert_eq!(registrar.parts.entity_selectors.len(), 7);
        assert_eq!(registrar.entity_selector_bytes, 229460);
        assert_eq!(registrar.refused.len(), 1);
        assert!(registrar.refused[0].contains("@t7") && registrar.refused[0].contains("256 KiB"));
        assert_eq!(
            registrar
                .parts
                .entity_selectors
                .iter()
                .map(|info| info.name.as_ref())
                .collect::<Vec<_>>(),
            ["t0", "t1", "t2", "t3", "t4", "t5", "t6"]
        );
    }

    #[test]
    pub(crate) fn registration_refuses_vanilla_invalid_and_duplicate_tokens() {
        /// A token that keeps every candidate.
        struct Everyone(u32);

        impl crate::EntitySelectorType for Everyone {
            fn base(&self) -> crate::SelectorBase {
                crate::SelectorBase::new(
                    true,
                    Some(self.0),
                    crate::SelectorSort::Nearest,
                    "test.selector.allies",
                )
            }

            fn matches(
                &self,
                _: &mut dyn crate::ServerHost,
                _: &crate::CommandSource,
                _: crate::EntityRef,
            ) -> bool {
                true
            }
        }

        struct Allies;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Allies {
            fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
                r.register_entity_selector("allies", Everyone(1));
            }
        }

        struct Invalid;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Invalid {
            fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
                r.register_entity_selector("allies", Everyone(1));
                r.register_entity_selector("allies", Everyone(1));
                r.register_entity_selector("a", Everyone(1));
                r.register_entity_selector("Bad", Everyone(1));
                r.register_entity_selector("zero", Everyone(0));
            }
        }

        crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
            "test",
            Box::new(Allies),
        )])
        .unwrap();
        let installed = mod_vocab::__private::parts_installation::testing_entity_selectors();
        assert_eq!(installed.len(), 1);
        assert_eq!(&*installed[0].name, "allies");
        assert_eq!(installed[0].selector.base().limit, Some(1));

        let errors = crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
            "test",
            Box::new(Invalid),
        )])
        .unwrap_err();
        assert_eq!(errors.len(), 4);
        assert!(errors[0].contains("@allies twice"));
        assert!(errors[1].contains("vanilla's selector @a"));
        assert!(errors[2].contains("invalid selector token @Bad"));
        assert!(errors[3].contains("@zero with zero limit"));
    }
}
