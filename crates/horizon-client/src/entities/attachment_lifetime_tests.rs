use super::*;
use crate::{
    entity_nbt,
    join_seq::{JoinList, Joined},
    mobs::Mob,
    projectiles::{Projectile, ProjectileKind},
};
use std::sync::{Arc, Mutex};
use {
    horizon_core::nbt::Tag,
    horizon_mod_runtime::attachments::{
        AttachmentLifetime, AttachmentType, Attachments, HasAttachmentLifetime, Holder,
        index::AttachmentIndex,
    },
};

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "entity_lifetime",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

fn exercise<T: Clone + Joined + HasAttachmentLifetime>(template: T) {
    for removal in ["retain", "clear", "drain", "owned_vec", "overwrite"] {
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut map = Attachments::default();
        VALUE.set(&mut map, Holder::Level, 10).unwrap();
        let mut entity = template.clone();
        *entity.attachment_lifetime() = AttachmentLifetime::new(map.bind_index(&index, 1u32));
        let mut list = JoinList::<T>::default();
        list.push(entity);
        let snapshot = || {
            index
                .lock()
                .unwrap()
                .snapshot(&("test".into(), "entity_lifetime".into()))
        };
        // A detached list clone neither retires the original nor holds its lease alive.
        let kept_clone = list.clone();
        drop(list.clone());
        assert_eq!(snapshot().len(), 1);
        let old = snapshot();
        let moved: Vec<T> = list.into_iter().collect();
        let mut list: JoinList<T> = moved.into_iter().collect();
        VALUE.set(&mut map, Holder::Level, 20).unwrap();
        assert_eq!(snapshot()[0].entry.read(&VALUE), Ok(&20));
        match removal {
            "retain" => list.retain(|_| false),
            "clear" => list.clear(),
            "drain" => drop(list.drain(..)),
            "owned_vec" => drop(std::mem::take(&mut list).into_iter().collect::<Vec<_>>()),
            "overwrite" => list[0] = template.clone(),
            _ => unreachable!(),
        }
        assert!(snapshot().is_empty(), "{removal}");
        assert_eq!(old[0].holder, 1);
        assert_eq!(old[0].entry.read(&VALUE), Ok(&10));
        VALUE.set(&mut map, Holder::Level, 30).unwrap();
        assert!(snapshot().is_empty());
        drop(kept_clone);
        assert!(snapshot().is_empty());
    }
    eprintln!(
        "{} = {} bytes; lifetime field = {} bytes",
        std::any::type_name::<T>(),
        size_of::<T>(),
        size_of::<AttachmentLifetime>()
    );
}

#[test]
fn reusable_entity_lifetimes_cover_clone_moves_and_vec_destruction() {
    let saved = Tag::compound([
        ("Pos", Tag::List(vec![Tag::Double(0); 3])),
        (
            "Item",
            Tag::compound([
                ("id", Tag::String("minecraft:stone".into())),
                ("count", Tag::Int(1)),
            ]),
        ),
    ]);
    exercise(entity_nbt::item_entity_from_nbt(&saved, 1, 0.0).unwrap());
    exercise(entity_nbt::orbs_from_nbt(&saved).pop().unwrap());
    exercise(ArmorStand::new(1, Vec3::ZERO, 0.0));
    exercise(Painting {
        attachment_lifetime: Default::default(),
        join: Default::default(),
        id: 1,
        pos: [0; 3],
        facing: 0,
        variant: "minecraft:kebab".into(),
        width: 1,
        height: 1,
        check_interval: 0,
    });
    exercise(entity_nbt::cushion_from_nbt(&saved, 1).unwrap());
    exercise(entity_nbt::item_frame_from_nbt(&saved, 1).unwrap());
    exercise(Mob::new(
        1,
        crate::mobs::sensor_scan_tests::kind(),
        Vec3::ZERO,
        0.0,
    ));
    exercise(Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None));
}

#[test]
fn projectile_equality_ignores_its_live_lifetime_but_clone_does_not_retire_it() {
    let index = Arc::new(Mutex::new(AttachmentIndex::default()));
    let mut map = Attachments::default();
    VALUE.set(&mut map, Holder::Level, 10).unwrap();
    let mut projectile = Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None);
    *projectile.attachment_lifetime() = AttachmentLifetime::new(map.bind_index(&index, 1u32));
    let clone = projectile.clone();
    assert_eq!(projectile, clone);
    drop(clone);
    let key = ("test".into(), "entity_lifetime".into());
    assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
    drop(projectile);
    assert!(index.lock().unwrap().snapshot(&key).is_empty());
}
