#[cfg(test)]
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, BackendRef, Host, HostBlockStates};
    struct MetadataHost(&'static horizon_mod_runtime::block_definitions::Blocks);
    impl Backend for MetadataHost {}
    crate::impl_backend_areas!(MetadataHost, except(block_states));
    impl crate::prelude::BlockStatesBackend for MetadataHost {
        fn block_metadata_name(&self, block: BlockKey) -> Result<&str, BlockStateError> {
            self.0.name(block)
        }
        fn block_metadata_property(
            &self,
            state: BlockStateRecord,
            name: &str,
        ) -> Result<Option<&str>, BlockStateError> {
            self.0.property(state, name)
        }
        fn block_metadata_with(
            &self,
            state: BlockStateRecord,
            name: &str,
            value: &str,
        ) -> Result<BlockStateRecord, BlockStateError> {
            self.0.with(state, name, value)
        }
        fn block_metadata_of(
            &self,
            block: BlockKey,
            properties: &[StateConstraint],
        ) -> Result<BlockStateRecord, BlockStateError> {
            self.0.of(block, properties)
        }
    }
    impl Host for MetadataHost {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(mod_vocab::__private::parts_installation::backend_ref(self))
        }
    }
    crate::impl_host_areas!(MetadataHost);
    struct Empty;
    impl Host for Empty {}
    crate::impl_host_areas!(Empty);

    #[test]
    fn metadata_service_reads_edits_and_refuses_invalid_transport() {
        crate::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let states = [
            std::collections::BTreeMap::from([("lit".into(), "false".into())]),
            std::collections::BTreeMap::from([("lit".into(), "true".into())]),
        ];
        crate::__private::resolve_block_keys(["probe:lamp"], |_| {
            horizon_mod_runtime::block_definitions::BlockDefinition::new(
                &[&states[0], &states[1]],
                0,
            )
            .map_err(|e| format!("{e:?}"))
        })
        .unwrap();
        let host = MetadataHost(crate::__private::block_definitions().unwrap());
        let metadata = host.block_states();
        let block = crate::__private::block_key("probe:lamp").unwrap();
        assert_eq!(metadata.name(block), Ok("probe:lamp"));
        assert_eq!(
            metadata.name(mod_vocab::__private::parts_installation::block_key(999)),
            Err(BlockStateError::UnknownBlock)
        );
        assert_eq!(
            Empty.block_states().name(block),
            Err(BlockStateError::Unavailable)
        );
        let off = metadata.of(block, &[]).unwrap();
        assert_eq!(off.state, 0);
        let on = metadata.with(off, "lit", "true").unwrap();
        assert_eq!(on.state, 1);
        assert_eq!(metadata.property(on, "lit"), Ok(Some("true")));
        assert_eq!(metadata.property(off, "lit"), Ok(Some("false")));
        let borrowed = crate::__private::backend_read(&host, |backend| {
            backend.block_metadata_property(off, "lit")
        })
        .unwrap()
        .unwrap();
        assert_eq!(borrowed, Some("false"));
        let value = crate::BlockState::of(&metadata, block, &[]).unwrap();
        let changed = value.with(&metadata, "lit", "true").unwrap();
        assert_eq!(value.property(&metadata, "lit"), Ok(Some("false")));
        assert_eq!(changed.property(&metadata, "lit"), Ok(Some("true")));
        assert_eq!(changed.ordinal(), 1);
        let states = crate::__private::block_state_range(block).unwrap();
        assert_eq!(
            states
                .iter()
                .map(|state| state.property(&metadata, "lit").unwrap().unwrap())
                .collect::<Vec<_>>(),
            ["false", "true"]
        );
        assert_eq!(
            crate::__private::block_state_range(
                mod_vocab::__private::parts_installation::block_key(999)
            ),
            Err(BlockStateError::UnknownBlock)
        );
        assert_eq!(
            changed.with(&metadata, "lit", "invalid"),
            Err(BlockStateError::InvalidValue(0))
        );
        assert_eq!(
            value.property(&Empty.block_states(), "lit"),
            Err(BlockStateError::Unavailable)
        );
        assert_eq!(metadata.property(off, "missing"), Ok(None));
        assert_eq!(
            metadata.with(off, "lit", "bad"),
            Err(BlockStateError::InvalidValue(0))
        );
        assert_eq!(
            metadata.property(BlockStateRecord::new(block, 2), "lit"),
            Err(BlockStateError::UnknownState)
        );
        assert_eq!(
            metadata.property(
                BlockStateRecord::new(mod_vocab::__private::parts_installation::block_key(999), 0),
                "lit"
            ),
            Err(BlockStateError::UnknownBlock)
        );
        assert_eq!(
            Empty.block_states().property(off, "lit"),
            Err(BlockStateError::Unavailable)
        );
    }
}
