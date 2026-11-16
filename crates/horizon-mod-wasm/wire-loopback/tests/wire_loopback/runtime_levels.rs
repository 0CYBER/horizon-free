use super::*;
use mod_api::prelude::*;
use mod_api::{LevelRef, RuntimeGenerator, RuntimeLevel, RuntimeLevelError};

#[derive(Default)]
struct World {
    generation: u32,
    open: bool,
    deletes: u32,
}
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World);
impl mod_api::ServerHost for World {}
mod_api::impl_server_host_areas!(World, except(services));
impl ServicesServerHost for World {
    fn runtime_levels(&mut self) -> &mut dyn mod_api::RuntimeLevels {
        self
    }
}
impl mod_api::RuntimeLevels for World {
    fn open(&mut self, id: &str, level: RuntimeLevel) -> Result<LevelRef, RuntimeLevelError> {
        assert_eq!(id, "wire:room");
        if self.deletes == 3 {
            return Err(RuntimeLevelError::Removing);
        }
        assert_eq!(level.dimension_type, "minecraft:overworld");
        assert_eq!(level.generator, RuntimeGenerator::Void);
        assert_eq!(level.seed, -41);
        if self.open {
            return Err(RuntimeLevelError::AlreadyOpen);
        }
        self.generation += 1;
        self.open = true;
        Ok(LevelRef::with_generation(
            mod_api::WorldEpoch::new(7),
            mod_api::DimensionKey::OVERWORLD,
            self.generation,
        ))
    }
    fn unload(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        assert_eq!(id, "wire:room");
        if !self.open {
            return Err(RuntimeLevelError::NotOpen);
        }
        self.open = false;
        Ok(())
    }
    fn delete(&mut self, id: &str) -> Result<(), RuntimeLevelError> {
        assert_eq!(id, "wire:room");
        self.deletes += 1;
        // The first answers as a level with a player in it does, the second as one with a level
        // below it.
        if self.deletes == 1 {
            return Err(RuntimeLevelError::PlayersInside);
        }
        if self.deletes == 2 {
            return Err(RuntimeLevelError::LevelsBelow);
        }
        self.open = false;
        Ok(())
    }
    fn is_open(&mut self, id: &str) -> bool {
        assert_eq!(id, "wire:room");
        self.open
    }
}

#[test]
fn runtime_level_hostcalls_preserve_requests_refusals_and_generations() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut world = World::default();
    let mut frame = boundary::Frame::default();
    frame.set_server_host_mut(&mut world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: world and its frame live throughout the synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let host = &mut guest::Guest;
    let spec = || RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Void, -41);
    assert!(!host.runtime_levels().is_open("wire:room"));
    let first = host.runtime_levels().open("wire:room", spec()).unwrap();
    assert_eq!(first.generation(), 1);
    assert_eq!(first.epoch().get(), 7);
    assert_eq!(
        host.runtime_levels().open("wire:room", spec()),
        Err(RuntimeLevelError::AlreadyOpen)
    );
    assert!(host.runtime_levels().is_open("wire:room"));
    assert_eq!(host.runtime_levels().unload("wire:room"), Ok(()));
    assert!(!host.runtime_levels().is_open("wire:room"));
    assert_eq!(
        host.runtime_levels().unload("wire:room"),
        Err(RuntimeLevelError::NotOpen)
    );
    let second = host.runtime_levels().open("wire:room", spec()).unwrap();
    assert_eq!(second.generation(), 2);
    assert_ne!(first, second);
    assert_eq!(
        first.generation(),
        1,
        "the old handle never adopts the reopened generation"
    );
    // A delete crosses the wire as its own call, and so does its refusal.
    assert_eq!(
        host.runtime_levels().delete("wire:room"),
        Err(RuntimeLevelError::PlayersInside)
    );
    assert!(host.runtime_levels().is_open("wire:room"));
    assert_eq!(
        host.runtime_levels().delete("wire:room"),
        Err(RuntimeLevelError::LevelsBelow)
    );
    assert_eq!(host.runtime_levels().delete("wire:room"), Ok(()));
    assert!(!host.runtime_levels().is_open("wire:room"));
    assert_eq!(
        host.runtime_levels().open("wire:room", spec()),
        Err(RuntimeLevelError::Removing)
    );
    guest::set_transport(None);
    assert_eq!(world.generation, 2);
    assert_eq!(world.deletes, 3);
    assert!(!world.open);
}
