//! The six world-mutation handle operations and their structured refusals across Wire.
use super::*;
use mod_api::__private::game::{BackendMutRefOf, BackendRefOf};
use mod_api::prelude::*;
use mod_api::{BlockState, ChunkRef, ChunkRefusal, LevelRef, PlayerRef, PlayerWorldRefusal};

#[derive(Default)]
struct World {
    chunks: Vec<[i32; 2]>,
    acknowledgements: Vec<(u128, i32)>,
    shown: Vec<(u128, [i32; 3], BlockState)>,
    placed: Vec<(u128, LevelRef, [i32; 3], BlockState, bool)>,
}
impl mod_api::Backend for World {}
mod_api::impl_backend_areas!(World, except(players));
impl PlayersClientBackend for World {}
impl PlayersBackend for World {
    fn player_sees(
        &self,
        viewer: PlayerRef,
        target: PlayerRef,
    ) -> Result<bool, PlayerWorldRefusal> {
        if viewer.uuid() != 7 {
            return Err(PlayerWorldRefusal::NoConnection);
        }
        Ok(target.uuid() == 8)
    }
}
impl mod_api::BackendMut for World {}
mod_api::impl_backend_mut_areas!(World, except(levels, players));
impl LevelsBackendMut for World {
    fn level_relight_chunks(
        &mut self,
        _: LevelRef,
        chunks: &[[i32; 2]],
    ) -> Result<u32, ChunkRefusal> {
        // A queue-limited maximum, smaller than the per-call limit of 256.
        if chunks.len() > 1 {
            return Err(ChunkRefusal::TooManyChunks {
                asked: chunks.len() as u32,
                max: 1,
            });
        }
        self.chunks.extend_from_slice(chunks);
        Ok(9)
    }
    fn chunk_resend(&mut self, chunk: ChunkRef) -> Result<(), ChunkRefusal> {
        if chunk.pos() == [2, -3] {
            Ok(())
        } else {
            Err(ChunkRefusal::NotLoaded)
        }
    }
}
impl PlayersBackendMut for World {
    fn player_ack_block_changes(
        &mut self,
        player: PlayerRef,
        sequence: i32,
    ) -> Result<(), PlayerWorldRefusal> {
        if sequence < 0 {
            return Err(PlayerWorldRefusal::NegativeSequence { sequence });
        }
        self.acknowledgements.push((player.uuid(), sequence));
        Ok(())
    }
    fn player_show_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        state: BlockState,
    ) -> Result<(), PlayerWorldRefusal> {
        if pos[1] == 2048 {
            return Err(PlayerWorldRefusal::PositionNotSendable { pos });
        }
        self.shown.push((player.uuid(), pos, state));
        Ok(())
    }
    fn player_place_block(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        shapes: bool,
    ) -> Result<(), PlayerWorldRefusal> {
        if pos[0] == 30_000_000 {
            return Err(PlayerWorldRefusal::OutOfWorld { pos });
        }
        if shapes {
            return Err(PlayerWorldRefusal::NotLoaded { pos });
        }
        self.placed.push((player.uuid(), level, pos, state, shapes));
        Ok(())
    }
}
impl mod_api::Host for World {
    fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
        Some(mod_api::BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(World);
impl mod_api::ServerHost for World {
    fn backend_mut(&mut self) -> Option<mod_api::BackendMutRef<'_>> {
        Some(mod_api::BackendMutRef::of(self))
    }
}
mod_api::impl_server_host_areas!(World);

#[test]
fn world_mutation_arguments_results_and_refusal_payloads_cross_the_wire() {
    mod_api::__private::initialize().unwrap();
    install_counter();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut world = World::default();
    let mut frame = boundary::Frame::default();
    frame.set_server_host_mut(&mut world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: the world and frame stay alive through this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let epoch = mod_api::WorldEpoch::new(7);
    let level = LevelRef::new(epoch, mod_api::DimensionKey::OVERWORLD);
    let player =
        |uuid| mod_api::__private::game::player_ref(mod_api::EntityRef::new(epoch, 5), uuid);
    let state = BlockState::from(mod_api::BlockStateRecord::new(
        REGISTERED_BLOCK.get().unwrap(),
        0,
    ));
    let host = &mut guest::Guest;
    assert_eq!(level.relight_chunks(host, &[[2, -3]]), Ok(9));
    assert_eq!(
        level.relight_chunks(host, &[[2, -3], [3, -3]]),
        Err(ChunkRefusal::TooManyChunks { asked: 2, max: 1 })
    );
    assert_eq!(ChunkRef::new(level, [2, -3]).resend(host), Ok(()));
    assert_eq!(
        ChunkRef::new(level, [9, 9]).resend(host),
        Err(ChunkRefusal::NotLoaded)
    );
    assert_eq!(player(7).sees(host, player(8)), Ok(true));
    assert_eq!(player(7).sees(host, player(9)), Ok(false));
    assert_eq!(
        player(9).sees(host, player(8)),
        Err(PlayerWorldRefusal::NoConnection)
    );
    assert_eq!(player(7).ack_block_changes(host, 17), Ok(()));
    assert_eq!(
        player(7).ack_block_changes(host, -2),
        Err(PlayerWorldRefusal::NegativeSequence { sequence: -2 })
    );
    assert_eq!(player(7).show_block(host, [-4, 64, 3], state), Ok(()));
    assert_eq!(
        player(7).show_block(host, [1, 2048, 2], state),
        Err(PlayerWorldRefusal::PositionNotSendable { pos: [1, 2048, 2] })
    );
    assert_eq!(
        player(7).place_block(host, level, [-4, 64, 3], state, false),
        Ok(())
    );
    assert_eq!(
        player(7).place_block(host, level, [5, 65, -6], state, true),
        Err(PlayerWorldRefusal::NotLoaded { pos: [5, 65, -6] })
    );
    assert_eq!(
        player(7).place_block(host, level, [30_000_000, 64, 0], state, false),
        Err(PlayerWorldRefusal::OutOfWorld {
            pos: [30_000_000, 64, 0]
        })
    );
    guest::set_transport(None);
    assert_eq!(world.chunks, [[2, -3]]);
    assert_eq!(world.acknowledgements, [(7, 17)]);
    assert_eq!(world.shown, [(7, [-4, 64, 3], state)]);
    assert_eq!(world.placed, [(7, level, [-4, 64, 3], state, false)]);
}
