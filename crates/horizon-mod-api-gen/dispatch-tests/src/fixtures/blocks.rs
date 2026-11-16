//! Declaration dispatch isolates each mod before applying the ownership boundary.
mod stubs {
    // This fixture tests attribution and applying/refusing boundary results, not
    // the namespace validator (the API's own tests exercise that boundary).
    pub(super) mod declaration_ownership {
        use crate::tests::mod_api::mod_api::{Block, Item};
        pub(in crate::tests) trait Declaration {
            fn id(&mut self) -> &mut String;
        }
        impl Declaration for Block {
            fn id(&mut self) -> &mut String {
                &mut self.id
            }
        }
        impl Declaration for Item {
            fn id(&mut self) -> &mut String {
                &mut self.id
            }
        }
        pub(in crate::tests) fn own_declaration(
            owner: &str,
            declaration: &mut impl Declaration,
        ) -> Result<(), String> {
            let id = declaration.id();
            super::super::DECLARATION_OWNERS.with_borrow_mut(|calls| {
                calls.push((owner.to_owned(), id.clone()));
            });
            if id == "reject" {
                Err(format!("{owner}: refused declaration"))
            } else {
                *id = format!("{owner}/checked/{id}");
                Ok(())
            }
        }
    }
}
use stubs::declaration_ownership as parts;
thread_local! {
    static DECLARATION_OWNERS: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
    static DECLARATION_REFUSALS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}
impl RegistrationContext {
    fn refuse_declaration(&self, reason: String) {
        DECLARATION_REFUSALS.with_borrow_mut(|errors| errors.push(reason));
    }
}

#[test]
fn block_batches_preserve_owner_normalization_and_invalid_output() {
    struct Declares(&'static str);
    #[mod_api::__subscriptions]
    impl Hooks for Declares {
        fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
            assert!(event.blocks.is_empty());
            event
                .blocks
                .push(mod_api::mod_api::Block { id: self.0.into() });
        }
    }
    install(
        vec![
            ModEntry::new("first", Box::new(Declares("first:pot"))),
            ModEntry::new("second", Box::new(Declares("reject"))),
        ]
        .into(),
    );
    DECLARATION_OWNERS.with_borrow_mut(Vec::clear);
    DECLARATION_REFUSALS.with_borrow_mut(Vec::clear);
    let mut blocks = vec![mod_api::mod_api::Block {
        id: "existing".into(),
    }];
    dispatch_blocks(&RegistrationContext("bootstrap"), &mut blocks);
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.id.as_str())
            .collect::<Vec<_>>(),
        ["existing", "first/checked/first:pot", "reject"]
    );
    DECLARATION_OWNERS.with_borrow(|calls| {
        assert_eq!(
            calls,
            &[
                ("first".into(), "first:pot".into()),
                ("second".into(), "reject".into())
            ]
        )
    });
    DECLARATION_REFUSALS.with_borrow(|errors| assert_eq!(errors, &["second: refused declaration"]));
}
