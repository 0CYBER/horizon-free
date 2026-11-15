#[derive(Debug, PartialEq, Eq)]
pub(in crate::game) enum BlockEntitySnapshotError {
    InvalidRevision,
    InvalidStack,
}

fn apply_block_entity_snapshot(
    entity: &mut crate::containers::ModBlockEntity,
    snapshot: &mod_api::ModBlockEntity,
    before: mod_api::item::containers::ModRevision,
    items: &Items,
) -> Result<(), BlockEntitySnapshotError> {
    let current = entity.revision();
    let revision = snapshot.revision();
    if snapshot.id() != entity.id()
        || revision.identity() != current.identity()
        || revision.changes() < current.changes()
    {
        return Err(BlockEntitySnapshotError::InvalidRevision);
    }
    if revision != before {
        let fields = (**snapshot)
            .try_into_engine(items)
            .map_err(|_| BlockEntitySnapshotError::InvalidStack)?;
        entity.apply_snapshot(fields, snapshot.revision().changes());
    }
    if let Some(fields) = snapshot.client_update_fields() {
        entity.set_client_update_fields(fields.clone());
    }
    Ok(())
}

// Validation runs after all listeners return; dispatch does not retain per-mod provenance.
// Bound diagnostics even when mods register many entity types.
pub(in crate::game) fn finish_block_entity_writeback<T>(
    entity: &mut crate::containers::ModBlockEntity,
    snapshot: &mod_api::ModBlockEntity,
    before: mod_api::item::containers::ModRevision,
    items: &Items,
    hook: &'static str,
    output: Result<T, BlockEntitySnapshotError>,
) -> Result<T, BlockEntitySnapshotError> {
    let result = output.and_then(|output| {
        apply_block_entity_snapshot(entity, snapshot, before, items)?;
        Ok(output)
    });
    if let Err(reason) = &result {
        let changes = entity
            .revision()
            .changes()
            .max(snapshot.revision().changes())
            .saturating_add(1);
        entity.apply_snapshot((**entity).clone(), changes);
        static REPORTED: std::sync::Mutex<Vec<(String, &'static str)>> =
            std::sync::Mutex::new(Vec::new());
        let mut reported = REPORTED.lock().unwrap_or_else(|poison| poison.into_inner());
        if reported.len() < 1024
            && !reported
                .iter()
                .any(|(id, name)| id == entity.id() && *name == hook)
        {
            reported.push((entity.id().to_owned(), hook));
            log::warn!(
                "Rejected block entity writeback: entity type {}, hook {}, reason {:?}",
                entity.id(),
                hook,
                reason
            );
        }
    }
    result
}
