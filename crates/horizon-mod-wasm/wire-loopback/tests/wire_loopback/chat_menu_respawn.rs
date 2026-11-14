//! `chat_recipient`, `menu_set_items` and `PlayerRef::respawn` through the generated host/guest
//! dispatch: the guest hook's answer, and each host call's arguments and named refusals.
use super::*;
use mod_api::__private::game::BackendMutRefOf;
use mod_api::prelude::*;
use mod_api::{
    ChatRecipientEvent, Flow, ItemStack, MenuItemsRefusal, RespawnRefusal, StackWriteError,
};

/// The guest side of `chat_recipient`: hides lines that say "secret" from receiver 13 alone.
pub(super) struct ChatRecipientHooks;
impl ChatRecipientHooks {
    pub(super) fn chat_recipient(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ChatRecipientEvent<'_>,
    ) -> mod_api::Flow {
        if event.message.contains("secret") && event.recipient == 13 {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}

/// The game's answers: the menu of player 123, window 5, holds two slots; player 7 is dead.
#[derive(Default)]
struct World {
    set: Vec<(u128, i32, Vec<Option<ItemStack>>)>,
    respawned: Vec<u128>,
}
impl mod_api::Backend for World {}
mod_api::impl_backend_areas!(World);
impl mod_api::BackendMut for World {}
mod_api::impl_backend_mut_areas!(World, except(players));
impl MenusServerHost for World {
    fn menu_set_items(
        &mut self,
        uuid: u128,
        window: i32,
        items: Vec<Option<ItemStack>>,
    ) -> Result<(), MenuItemsRefusal> {
        if uuid != 123 {
            return Err(MenuItemsRefusal::NoPlayer);
        }
        if window != 5 {
            return Err(MenuItemsRefusal::NotOpen);
        }
        if items.iter().flatten().any(|stack| stack.count() > 16) {
            return Err(MenuItemsRefusal::Stack(StackWriteError::CountExceeded {
                maximum: 16,
            }));
        }
        self.set.push((uuid, window, items));
        Ok(())
    }
}
impl PlayersBackendMut for World {
    fn player_respawn(&mut self, player: mod_api::PlayerRef) -> Result<(), RespawnRefusal> {
        match player.uuid() {
            7 => {
                self.respawned.push(7);
                Ok(())
            }
            8 => Err(RespawnRefusal::Alive),
            _ => Err(RespawnRefusal::Stale),
        }
    }
}
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World);
impl mod_api::ServerHost for World {
    fn backend_mut(&mut self) -> Option<mod_api::BackendMutRef<'_>> {
        Some(mod_api::BackendMutRef::of(self))
    }
}
mod_api::impl_server_host_areas!(World, except(menus));

#[test]
fn chat_recipient_hides_one_receiver_through_the_wire() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    install_counter();
    let module = WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    };
    let ask = |message: &str, recipient: u128| {
        let mut event = ChatRecipientEvent {
            sender: 12,
            sender_name: "guest12",
            message,
            recipient,
            cancelled: false,
        };
        module.chat_recipient(&mut World::default(), &mut event)
    };
    assert!(matches!(ask("a secret", 13), Flow::Cancel));
    assert!(matches!(ask("a secret", 12), Flow::Continue));
    assert!(matches!(ask("plain", 13), Flow::Continue));
}

#[test]
fn menu_set_items_and_respawn_cross_the_wire_with_their_named_refusals() {
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
        // SAFETY: world and its frame remain live for this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let key = REGISTERED_ITEM.get().unwrap();
    let two = ItemStack::new(key, 2).unwrap();
    let host = &mut guest::Guest;
    assert_eq!(
        host.menu_set_items(123, 5, vec![Some(two.clone()), None]),
        Ok(())
    );
    assert_eq!(
        host.menu_set_items(999, 5, vec![]),
        Err(MenuItemsRefusal::NoPlayer)
    );
    assert_eq!(
        host.menu_set_items(123, 6, vec![]),
        Err(MenuItemsRefusal::NotOpen)
    );
    assert_eq!(
        host.menu_set_items(123, 5, vec![Some(ItemStack::new(key, 17).unwrap())]),
        Err(MenuItemsRefusal::Stack(StackWriteError::CountExceeded {
            maximum: 16
        }))
    );
    let epoch = mod_api::WorldEpoch::new(7);
    let player =
        |uuid: u128| mod_api::__private::game::player_ref(mod_api::EntityRef::new(epoch, 5), uuid);
    assert_eq!(player(7).respawn(host), Ok(()));
    assert_eq!(player(8).respawn(host), Err(RespawnRefusal::Alive));
    assert_eq!(player(9).respawn(host), Err(RespawnRefusal::Stale));
    guest::set_transport(None);
    assert_eq!(world.set, vec![(123, 5, vec![Some(two), None])]);
    assert_eq!(world.respawned, vec![7]);
}
