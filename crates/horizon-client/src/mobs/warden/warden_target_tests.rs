//! `Warden.tick`'s dig cooldown for a persistence-required warden and
//! `Warden.canTargetEntity`, which leaves its allies and invulnerable
//! entities alone.

use super::*;
use crate::mobs::world::{test_player, with_test_world};
use crate::mobs::{Seen, Teams, leash};
use crate::save::uuid_text;
use crate::scoreboard::Scoreboard;
use std::sync::Arc;

const WARDEN_UUID: [i32; 4] = [1, 2, 3, 4];
const ZOMBIE_UUID: [i32; 4] = [5, 6, 7, 8];

fn warden_mob(types: &crate::mobs::MobTypes, id: u32) -> Mob {
    let kind = Arc::new(types.types["minecraft:warden"].clone());
    Mob::new(id, kind, Vec3::new(0.5, 64.0, 0.5), 0.0)
}

/// A zombie `id` as the mobs' snapshot sees it, invulnerable or not.
fn zombie(id: u32, invulnerable: bool) -> Seen {
    let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
    kind.species = Species::Zombie;
    let mut m = Mob::new(id, Arc::new(kind), Vec3::new(2.5, 64.0, 0.5), 0.0);
    m.invulnerable_flag = invulnerable;
    m.seen(None, &crate::items::Items::empty())
}

fn dig_cooldown(m: &mut Mob) -> i64 {
    warden_brain::mem(m).dig_cooldown.ttl()
}

/// `Warden.tick`: `isPersistenceRequired() || requiresCustomPersistence()`
/// refreshes the dig cooldown, which is what keeps a warden from digging away.
/// A custom name is neither of the two (`Mob.requiresCustomPersistence` is
/// "passenger or leashed"), so a `/summon`ed warden called Bob digs down and
/// is gone.
#[test]
fn only_a_persistent_or_leashed_warden_keeps_its_dig_cooldown() {
    let ran = with_test_world(&[], |w, types| {
        // What a warden holds a few ticks into its dig cooldown.
        let cases: [(&str, fn(&mut Mob), i64); 4] = [
            ("plain", |_| {}, 3),
            ("named", |m| m.custom_name = Some("Bob".into()), 3),
            ("persistent", |m| m.persistent = true, DIGGING_COOLDOWN),
            (
                "leashed",
                |m| m.extra.leash = Some(leash::Holder::Player),
                DIGGING_COOLDOWN,
            ),
        ];
        for (what, set, ttl) in cases {
            let mut m = warden_mob(types, 1);
            set(&mut m);
            warden_brain::mem(&mut m).dig_cooldown.set_ttl((), 3);
            pre_tick(&mut m, w, &mut Vec::new());
            assert_eq!(dig_cooldown(&mut m), ttl, "{what} warden");
        }
    });
    let _ = ran;
}

/// `Warden.canTargetEntity`: `!isAlliedTo(entity) && !entity.isInvulnerable()`.
/// A warden on a player's team leaves that player and its other allies alone,
/// and it never hunts an invulnerable mob.
#[test]
fn the_warden_leaves_its_allies_and_invulnerable_mobs_alone() {
    let mut board = Scoreboard::default();
    board.add_team("t");
    board.add_to_team("Steve", "t");
    board.add_to_team(&uuid_text(WARDEN_UUID), "t");
    board.add_to_team(&uuid_text(ZOMBIE_UUID), "t");
    let team_of = |uuid| board.players_team(&uuid_text(uuid)).expect("on team t");
    let warden_team = team_of(WARDEN_UUID);
    let zombie_team = team_of(ZOMBIE_UUID);
    // The warden alone on "t", and with the player and the zombie 2 on it.
    let alone = Teams {
        player: None,
        mobs: HashMap::from([(1, warden_team)]),
    };
    let allied = Teams {
        player: board.players_team("Steve"),
        mobs: HashMap::from([(1, warden_team), (2, zombie_team)]),
    };
    // The zombie 3 is invulnerable and on no team.
    let others = [zombie(2, false), zombie(3, true)];
    let player = test_player(Vec3::new(0.5, 64.0, 2.5));

    let ran = with_test_world(&others, |w, types| {
        let warden = warden_mob(types, 1);
        let mut w = w.clone();
        w.player = Some(player.clone());
        w.teams = Arc::new(allied.clone());
        assert!(!can_target(&w, warden.id, Who::Player), "an allied player");
        assert!(!can_target(&w, warden.id, Who::Mob(2)), "an allied mob");
        assert!(
            !can_target(&w, warden.id, Who::Mob(3)),
            "an invulnerable mob"
        );
    });
    // Nobody else on the warden's team: the same two are targetable.
    let ran = ran
        && with_test_world(&others, |w, types| {
            let warden = warden_mob(types, 1);
            let mut w = w.clone();
            w.player = Some(player);
            w.teams = Arc::new(alone.clone());
            assert!(can_target(&w, warden.id, Who::Player), "a stranger");
            assert!(can_target(&w, warden.id, Who::Mob(2)), "a stranger mob");
            assert!(
                !can_target(&w, warden.id, Who::Mob(3)),
                "an invulnerable mob"
            );
        });
    let _ = ran;
}

/// `Warden.VibrationUser.canReceiveVibration` through `isAlliedTo`: a team
/// mate's vibration is not one the warden hears, so it never angers at it.
#[test]
fn an_allied_vibration_is_not_heard() {
    let mut board = Scoreboard::default();
    board.add_team("t");
    board.add_to_team(&uuid_text(WARDEN_UUID), "t");
    board.add_to_team(&uuid_text(ZOMBIE_UUID), "t");
    let team_of = |uuid| board.players_team(&uuid_text(uuid)).expect("on team t");
    let teams = Teams {
        player: board.players_team("Steve"),
        mobs: HashMap::from([(1, team_of(WARDEN_UUID)), (2, team_of(ZOMBIE_UUID))]),
    };
    let ran = with_test_world(&[], |w, types| {
        let m = warden_mob(types, 1);
        let ctx = Context {
            source: Some(Who::Mob(2).into()),
            living: true,
            targetable: true,
            ..Context::default()
        };
        let ally = heard_by(ctx, &teams, m.id);
        assert!(!ally.targetable, "the team mate is not one it may target");
        assert!(!can_receive_vibration(&m, 0, &ally), "an ally is unheard");
        let stranger = heard_by(ctx, &w.teams, m.id);
        assert!(stranger.targetable);
        assert!(
            can_receive_vibration(&m, 0, &stranger),
            "a stranger is heard"
        );
        // A source on no team at all: `teams_allied` needs both sides.
        let none = heard_by(ctx, &Teams::default(), m.id);
        assert!(can_receive_vibration(&m, 0, &none));
    });
    let _ = ran;
}
