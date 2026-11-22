use crate::game::{Game, execution_role::ExecutionRole, native_guest_turn_tests::with_guests};
use horizon_client::{
    first_person::Hand,
    items::ItemStack,
    lan::{Out, PlayerSender, playerdata::uuid_ints},
    mobs::Mob,
};
use horizon_core::{geometry::Vec3, nbt::Tag};
use horizon_protocol::{buf::Reader, packets::play::Clientbound};
use mod_api::{
    EntityEnterLoveModeEvent, EntityEnterLoveModePreEvent, EntityRef, Flow, Hooks, ModEntry,
    ServerHost, Text,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

const QUOTA_MESSAGE: &str = "Breeding quota reached";

#[derive(Debug, PartialEq)]
struct Entry {
    animal: u32,
    kind: String,
    feeder: Option<u128>,
    ticks: i32,
}

#[derive(Clone, Default)]
struct LoveProbe {
    veto: Arc<AtomicBool>,
    entries: Arc<Mutex<Vec<Entry>>>,
}

impl LoveProbe {
    fn record(&self, host: &mut dyn ServerHost, animal: EntityRef, feeder: Option<u128>) {
        let kind = animal.kind(host).unwrap().id(host);
        let nbt = Tag::Compound(animal.nbt(host).unwrap());
        let Some(&Tag::Int(ticks)) = nbt.get("InLove") else {
            panic!("{kind}: animal's love timer should be readable");
        };
        self.entries.lock().unwrap().push(Entry {
            animal: animal.id(),
            kind,
            feeder,
            ticks,
        });
    }
}

#[mod_api::__subscriptions]
impl Hooks for LoveProbe {
    fn entity_enter_love_mode_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityEnterLoveModePreEvent,
    ) -> Flow {
        assert_eq!(event.level.dimension_id(), "minecraft:overworld");
        assert_eq!(host.level("minecraft:overworld"), Some(event.level));
        assert_eq!(host.players().acting_player(), event.feeder);
        self.record(host, event.animal, event.feeder.map(|feeder| feeder.uuid()));
        if self.veto.load(Ordering::Relaxed) {
            if let Some(feeder) = event.feeder {
                let message = Text::literal(QUOTA_MESSAGE);
                assert_eq!(feeder.tell(host, message), Ok(true));
            }
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn entity_enter_love_mode(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityEnterLoveModeEvent<'_>,
    ) {
        let animal = host.entity(event.entity).unwrap();
        assert_eq!(animal.kind(host).unwrap().id(host), event.kind);
        self.record(host, animal, event.player);
    }
}

fn feed(game: &mut Game<'_>, feeder: u32, id: u32, food: &str) {
    let gi = game.guest_index(feeder).unwrap();
    game.lan.as_mut().unwrap().hub.guests.update(gi, |guest| {
        guest.inventory[0] = Some(ItemStack::new(food, 2));
    });
    assert!(
        game.guest_interact(feeder, gi, id as i32, Hand::Main, [0.0; 3])
            .unwrap(),
        "the feeding click should be handled for {food}",
    );
}

fn received_quota_message(receiver: &mpsc::Receiver<Out>) -> bool {
    let expected = horizon_client::lan::data::component_nbt(
        &horizon_client::text::Text::literal(QUOTA_MESSAGE).component(),
    );
    let is_quota_message = |body: &[u8]| {
        let mut reader = Reader::new(body);
        let id = reader.varint().unwrap();
        matches!(
            Clientbound::decode(id, reader.rest()),
            Ok(Clientbound::SystemChat(chat)) if !chat.overlay && chat.content == expected
        )
    };
    let mut received = false;
    for message in receiver.try_iter() {
        received |= match message {
            Out::Packet(body) => is_quota_message(&body),
            Out::Packets(bodies) => bodies.iter().any(|body| is_quota_message(body)),
            Out::Disconnect(_) => panic!("a feeder was disconnected"),
        };
    }
    received
}

fn spawn_animal(game: &mut Game<'_>, kind: &str, age: i32) -> u32 {
    game.mobs.clear();
    game.next_entity += 1;
    let id = game.next_entity;
    let mob_type = game.res.mob_types().get(kind).unwrap().clone();
    let mut mob = Mob::new(id, mob_type, Vec3::new(4.5, 64.0, 5.0), 0.0);
    mob.extra.age = age;
    mob.extra.tame = true;
    mob.health = mob.attrs.max_health;
    game.mobs.push(mob);
    game.lan_entity_events.clear();
    id
}

fn food_left(game: &Game<'_>, feeder: u32) -> u32 {
    let hub = &game.lan.as_ref().unwrap().hub;
    hub.guest(feeder).unwrap().inventory[0]
        .as_ref()
        .unwrap()
        .count
}

fn check_love_feed(
    game: &mut Game<'_>,
    probe: &LoveProbe,
    receivers: &[mpsc::Receiver<Out>],
    kind: &str,
    food: &str,
) {
    let id = spawn_animal(game, kind, 0);
    let case = format!("{:?} {kind}", game.options.execution_role);
    let before = |feeder| Entry {
        animal: id,
        kind: kind.to_owned(),
        feeder: Some(feeder),
        ticks: 0,
    };

    probe.veto.store(true, Ordering::Relaxed);
    feed(game, 13, id, food);
    let animal = &game.mobs[0].extra;
    assert_eq!(animal.in_love, 0, "{case}");
    assert!(!animal.love_player, "{case}");
    assert_eq!(animal.love_cause, None, "{case}");
    assert_eq!(animal.age, 0, "{case}");
    assert!(!game.love_cause.contains_key(&id), "{case}");
    assert!(!game.lan_entity_events.contains(&(id as i32, 18)), "{case}");
    assert_eq!(food_left(game, 13), 1, "{case}: the animal ate the food");
    assert!(
        received_quota_message(&receivers[1]),
        "{case}: notify the feeder"
    );
    assert!(
        !received_quota_message(&receivers[0]),
        "{case}: leave the other player's chat alone"
    );
    assert_eq!(
        std::mem::take(&mut *probe.entries.lock().unwrap()),
        [before(13)],
        "{case}: veto only dispatches the pre-entry event",
    );

    probe.veto.store(false, Ordering::Relaxed);
    feed(game, 12, id, food);
    assert_eq!(game.mobs[0].extra.in_love, 600, "{case}");
    assert!(game.mobs[0].extra.love_player, "{case}");
    assert_eq!(game.love_cause.get(&id), Some(&Some(12)), "{case}");
    if kind == "minecraft:cow" {
        assert_eq!(game.mobs[0].extra.love_cause, Some(uuid_ints(12)), "{case}");
    }
    let hearts = game
        .lan_entity_events
        .iter()
        .filter(|&&event| event == (id as i32, 18));
    assert_eq!(hearts.count(), 1, "{case}");
    assert_eq!(food_left(game, 12), 1, "{case}");
    assert_eq!(
        std::mem::take(&mut *probe.entries.lock().unwrap()),
        [
            before(12),
            Entry {
                ticks: 600,
                ..before(12)
            }
        ],
        "{case}: the old observer sees the committed timer",
    );
}

#[test]
#[ignore = "daily"]
fn eligible_feeds_veto_only_love_mode_and_continue_with_the_real_feeder() {
    let probe = LoveProbe::default();
    let mods = vec![ModEntry::new("animal-love-probe", Box::new(probe.clone()))].into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            let mut receivers = Vec::new();
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.guests.update_all(|guest| {
                let (sender, receiver) = mpsc::channel();
                guest.sender = PlayerSender::new(sender, Arc::default());
                receivers.push(receiver);
            });
            game.audio.capture = Some(Vec::new());
            game.next_entity = 100;
            for role in [
                ExecutionRole::IntegratedServer,
                ExecutionRole::DedicatedServer,
            ] {
                game.options.execution_role = role;
                game.options.dedicated = (role == ExecutionRole::DedicatedServer).then(Vec::new);
                for (kind, food) in [
                    ("minecraft:cow", "minecraft:wheat"),
                    ("minecraft:turtle", "minecraft:seagrass"),
                    ("minecraft:horse", "minecraft:golden_carrot"),
                    ("minecraft:llama", "minecraft:hay_block"),
                    ("minecraft:panda", "minecraft:bamboo"),
                    ("minecraft:nautilus", "minecraft:cod"),
                ] {
                    check_love_feed(game, &probe, &receivers, kind, food);
                }
            }

            // An ordinary baby still takes food to grow, without proposing love mode.
            let id = spawn_animal(game, "minecraft:cow", -24_000);
            feed(game, 13, id, "minecraft:wheat");
            assert_eq!(game.mobs[0].extra.age, -21_600);
            assert_eq!(food_left(game, 13), 1);

            // An adult on its normal breeding cooldown does not propose entry either.
            let id = spawn_animal(game, "minecraft:cow", 6_000);
            feed(game, 13, id, "minecraft:wheat");
            assert_eq!(game.mobs[0].extra.age, 6_000);
            assert_eq!(food_left(game, 13), 2);
            assert!(probe.entries.lock().unwrap().is_empty());
        });
    });
}
