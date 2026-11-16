use super::*;
use mod_api::prelude::*;

/// A server whose upgrades mark what they were given: an entity gains `upgraded_from` (and is refused as upgraded
/// too deep from version 100), a block entity is refused as given too deep.
struct World;
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World);
impl mod_api::ServerHost for World {}
mod_api::impl_server_host_areas!(World, except(world_edit));
impl WorldEditServerHost for World {
    fn upgrade_entity_nbt(
        &self,
        data_version: i32,
        mut nbt: mod_api::Compound,
    ) -> Result<mod_api::Compound, mod_api::NbtUpgradeError> {
        if data_version == 100 {
            return Err(mod_api::NbtUpgradeError::UpgradedTooDeep);
        }
        nbt.insert("upgraded_from".into(), mod_api::Tag::Int(data_version));
        Ok(nbt)
    }
    fn upgrade_block_entity_nbt(
        &self,
        data_version: i32,
        _: mod_api::Compound,
    ) -> Result<mod_api::Compound, mod_api::NbtUpgradeError> {
        Err(if data_version < 0 {
            mod_api::NbtUpgradeError::NegativeVersion(data_version)
        } else {
            mod_api::NbtUpgradeError::TooDeep
        })
    }
}

#[test]
fn datafix_upgrades_round_trip_the_data_and_the_typed_refusals() {
    // The platform phase this scope sets is process-wide state another test may not have installed yet.
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let world = World;
    let mut frame = boundary::Frame::default();
    frame.set_server_host(&world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: world and its frame remain live for this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let mut villager = mod_api::Compound::new();
    villager.insert(
        "id".into(),
        mod_api::Tag::String("minecraft:villager".into()),
    );
    let mut expected = villager.clone();
    expected.insert("upgraded_from".into(), mod_api::Tag::Int(1900));
    assert_eq!(
        guest::Guest.upgrade_entity_nbt(1900, villager.clone()),
        Ok(expected)
    );
    assert_eq!(
        guest::Guest.upgrade_entity_nbt(100, villager.clone()),
        Err(mod_api::NbtUpgradeError::UpgradedTooDeep)
    );
    assert_eq!(
        guest::Guest.upgrade_block_entity_nbt(-1, villager.clone()),
        Err(mod_api::NbtUpgradeError::NegativeVersion(-1))
    );
    assert_eq!(
        guest::Guest.upgrade_block_entity_nbt(1900, villager),
        Err(mod_api::NbtUpgradeError::TooDeep)
    );
    guest::set_transport(None);
}
