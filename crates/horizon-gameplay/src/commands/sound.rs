//! `/playsound` and `/stopsound`.
use super::entity::{EntityHost, EntityRef, holder, num};
use super::host::R;
use horizon_client::{
    command::{Arg, Context},
    scoreboard_cmd::Tracker,
    settings::SOUND_SOURCES,
    text::Text,
};

/// What `/playsound` and `/stopsound` need from the game.
pub trait SoundHost: EntityHost {
    /// The player in place, who `/playsound` targets when it names no one.
    fn player_in_place(&self) -> EntityRef;
    /// Whether the entity is a player who hears sounds in the source's level.
    fn hears_sounds(&self, entity: EntityRef) -> bool;
    /// One draw from the level random, the seed of a sound packet.
    fn sound_seed(&mut self) -> i64;
    /// `ClientboundSoundPacket` to one player.
    fn play_sound_to(
        &mut self,
        entity: EntityRef,
        sound: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        source: usize,
        seed: i64,
    );
    /// `StopSoundCommand`: each target's connection gets the packet.
    fn stop_sound_for(&mut self, targets: &[EntityRef], sound: Option<&str>, source: Option<usize>);
}

/// `/playsound <sound> [<source>] [<targets>] [<pos>] [<volume>] [<pitch>] [<minVolume>]`.
pub fn playsound(host: &mut dyn SoundHost, ctx: &Context) -> R<i32> {
    let Some(Arg::Resource(sound)) = ctx.get("sound").cloned() else {
        return Ok(0);
    };
    let source = ctx
        .literals
        .iter()
        .find_map(|l| SOUND_SOURCES.iter().position(|s| s == l))
        .unwrap_or(0);
    let targets = if ctx.get("targets").is_some() {
        host.select_entities(ctx, "targets", true)?
    } else {
        vec![host.player_in_place()]
    };
    let pos = host
        .position_arg(ctx, "pos")
        .unwrap_or_else(|| host.source_position());
    let float = |name: &str, default: f32| match ctx.get(name) {
        Some(Arg::Float(v)) => *v,
        _ => default,
    };
    let (volume, pitch, min_volume) = (
        float("volume", 1.0),
        float("pitch", 1.0),
        float("minVolume", 0.0),
    );
    let range = if volume > 1.0 { 16.0 * volume } else { 16.0 };
    // PlaySoundCommand: one seed for every player.
    let seed = host.sound_seed();
    let mut tracker = Tracker::default();
    for t in targets {
        // Only players in the source's level hear it.
        if !host.hears_sounds(t) {
            continue;
        }
        let Some(p) = host.entity_position(t) else {
            continue;
        };
        let Some((at, v)) = playsound_placement(pos, p, range, volume, min_volume) else {
            continue;
        };
        host.play_sound_to(t, &sound, at, v, pitch, source, seed);
        tracker.mark(&holder(host.entity_name(t)));
    }
    let name = Text::literal(sound.clone());
    let (message, total) = tracker.message(
        true,
        Some(host.error("commands.playsound.failed", &[])),
        |h, _| {
            Text::translate(
                "commands.playsound.success.single",
                &[name.clone(), h.text.clone()],
            )
        },
        |c, _| {
            Text::translate(
                "commands.playsound.success.multiple",
                &[name.clone(), num(c)],
            )
        },
    )?;
    host.feedback(message);
    Ok(total)
}

/// `/stopsound <targets> [<source>] [<sound>]`.
pub fn stopsound(host: &mut dyn SoundHost, ctx: &Context) -> R<i32> {
    let targets = host.select_entities(ctx, "targets", true)?;
    let source = ctx
        .literals
        .iter()
        .find_map(|l| SOUND_SOURCES.iter().position(|s| s == l));
    let sound = match ctx.get("sound") {
        Some(Arg::Resource(s)) => Some(s.clone()),
        _ => None,
    };
    host.stop_sound_for(&targets, sound.as_deref(), source);
    let key = match (source, &sound) {
        (Some(_), Some(_)) => "commands.stopsound.success.source.sound",
        (Some(_), None) => "commands.stopsound.success.source.any",
        (None, Some(_)) => "commands.stopsound.success.sourceless.sound",
        (None, None) => "commands.stopsound.success.sourceless.any",
    };
    let mut args = Vec::new();
    if let Some(s) = &sound {
        args.push(Text::literal(s.clone()));
    }
    if let Some(i) = source {
        args.push(Text::literal(SOUND_SOURCES[i]));
    }
    host.feedback(Text::translate(key, &args));
    Ok(targets.len() as i32)
}

/// `PlaySoundCommand.playSound`: the point and the volume one target hears the
/// sound at. A target past `range` hears nothing unless `minVolume` is above
/// zero, and then the point moves two blocks from the target towards the sound
/// and the volume is `minVolume`; a target exactly at `range` still hears it
/// (the command's test is `>`, unlike `PlayerList.broadcast`'s `<`).
#[doc(hidden)]
pub fn playsound_placement(
    pos: [f64; 3],
    target: [f64; 3],
    range: f32,
    volume: f32,
    min_volume: f32,
) -> Option<([f64; 3], f32)> {
    let d = [pos[0] - target[0], pos[1] - target[1], pos[2] - target[2]];
    let dist2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    // PlaySoundCommand compares against Mth.square(float range): the square
    // rounds in float and only then widens, so a point between that square and
    // the double square is out of range in vanilla.
    if dist2 <= f64::from(range * range) {
        return Some((pos, volume));
    }
    if min_volume <= 0.0 {
        return None;
    }
    let dist = dist2.sqrt();
    Some((
        [
            target[0] + d[0] / dist * 2.0,
            target[1] + d[1] / dist * 2.0,
            target[2] + d[2] / dist * 2.0,
        ],
        min_volume,
    ))
}
