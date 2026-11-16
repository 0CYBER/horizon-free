use super::*;
use mod_api::__private::game::BackendMutRefOf;
use mod_api::prelude::*;
use mod_api::{ChunkArea, ChunkAreaMode, ChunkAreaRefusal, ChunkCap, ChunkPos, LevelRef};

/// The areas the game was asked for, by player UUID; it refuses as the hub would.
#[derive(Default)]
struct World(Vec<(u128, Vec<ChunkArea>)>);
impl mod_api::Backend for World {}
mod_api::impl_backend_areas!(World);
impl mod_api::BackendMut for World {}
mod_api::impl_backend_mut_areas!(World, except(players));
impl PlayersBackendMut for World {
    fn player_set_chunk_areas(
        &mut self,
        player: mod_api::PlayerRef,
        areas: &[ChunkArea],
    ) -> Result<(), ChunkAreaRefusal> {
        if player.uuid() == 99 {
            return Err(ChunkAreaRefusal::NotOnline);
        }
        if let Some(a) = areas.iter().find(|a| a.radius > 32) {
            return Err(ChunkAreaRefusal::RadiusTooLarge {
                radius: a.radius,
                max: 32,
            });
        }
        if let Some(a) = areas.iter().find(|a| a.center.x > 1_875_000) {
            return Err(ChunkAreaRefusal::CenterOutOfRange {
                center: a.center,
                max: 1_875_000,
            });
        }
        if let Some(a) = areas.iter().find(|a| a.mode == ChunkAreaMode::Ticking) {
            return Err(ChunkAreaRefusal::TooManyChunks {
                cap: ChunkCap::TickingPerLevel,
                level: a.level,
                asked: 3721,
                limit: 3528,
            });
        }
        self.0.push((player.uuid(), areas.to_vec()));
        Ok(())
    }
}
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World);
impl mod_api::ServerHost for World {
    fn backend_mut(&mut self) -> Option<mod_api::BackendMutRef<'_>> {
        Some(mod_api::BackendMutRef::of(self))
    }
}
mod_api::impl_server_host_areas!(World);

#[test]
fn chunk_areas_and_their_refusals_cross_the_wire() {
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
        // SAFETY: world and its frame remain live for this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let epoch = mod_api::WorldEpoch::new(7);
    let nether = LevelRef::new(epoch, mod_api::DimensionKey::NETHER);
    let player =
        |uuid: u128| mod_api::__private::game::player_ref(mod_api::EntityRef::new(epoch, 5), uuid);
    let area = |center: ChunkPos, radius, mode| ChunkArea {
        level: nether,
        center,
        radius,
        mode,
    };
    let loaded = area(ChunkPos::new(-40, 12), 3, ChunkAreaMode::Loaded);
    let host = &mut guest::Guest;
    assert_eq!(player(12).set_chunk_areas(host, &[loaded]), Ok(()));
    assert_eq!(player(12).set_chunk_areas(host, &[]), Ok(()));
    assert_eq!(
        player(99).set_chunk_areas(host, &[loaded]),
        Err(ChunkAreaRefusal::NotOnline)
    );
    assert_eq!(
        player(12).set_chunk_areas(
            host,
            &[area(ChunkPos::new(0, 0), 33, ChunkAreaMode::Loaded)]
        ),
        Err(ChunkAreaRefusal::RadiusTooLarge {
            radius: 33,
            max: 32
        })
    );
    assert_eq!(
        player(12).set_chunk_areas(
            host,
            &[area(ChunkPos::new(i32::MAX, -7), 0, ChunkAreaMode::Loaded)]
        ),
        Err(ChunkAreaRefusal::CenterOutOfRange {
            center: ChunkPos::new(i32::MAX, -7),
            max: 1_875_000
        })
    );
    assert_eq!(
        player(12).set_chunk_areas(
            host,
            &[
                loaded,
                area(ChunkPos::new(5, 5), 10, ChunkAreaMode::Ticking)
            ]
        ),
        Err(ChunkAreaRefusal::TooManyChunks {
            cap: ChunkCap::TickingPerLevel,
            level: nether,
            asked: 3721,
            limit: 3528
        })
    );
    guest::set_transport(None);
    assert_eq!(
        world.0,
        vec![(12, vec![loaded]), (12, Vec::new())],
        "the game got each accepted call's areas as the mod sent them"
    );
}
