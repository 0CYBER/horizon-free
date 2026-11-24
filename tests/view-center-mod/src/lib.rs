//! An in-game check of `PlayerRef::set_view_center` (horizon-rust/horizon#19219), as a camera mod
//! uses it. Chat `view <chunk x> <chunk z>`: the server streams the chunks around that chunk to
//! you, and your camera looks down on it from above, while you stay where you are. `view off`
//! puts both back on you. Works in single player, for a LAN host and for a LAN guest
//! (`tools/lan_e2e.py --check view_center`).
use mod_api::{
    CameraPoseEvent, ChunkPos, ClientChatReceivedEvent, ClientHost, Flow, Hooks, ServerChatEvent,
    ServerHost, Text, text::TextContent,
};
use std::sync::Mutex;

/// The chunk this window's camera looks at: the centre the server last accepted for the local
/// player, as its reply says. Each window keeps its own, so a guest's never moves the host's.
static CAMERA: Mutex<Option<(i32, i32)>> = Mutex::new(None);

/// The server's reply to a call it accepted: `view centre x z`, or `view centre off`.
const ACCEPTED: &str = "view centre ";

#[derive(Default)]
struct Fixture;

/// `view <x> <z>` as Some(Some(chunk)), `view off` as Some(None), anything else as None.
fn parse(text: &str) -> Option<Option<(i32, i32)>> {
    let mut words = text.split_whitespace();
    if words.next()? != "view" {
        return None;
    }
    match (words.next()?, words.next(), words.next()) {
        ("off", None, None) => Some(None),
        (x, Some(z), None) => Some(Some((x.parse().ok()?, z.parse().ok()?))),
        _ => None,
    }
}

/// The reply the server sends for a centre it accepted.
fn accepted(view: Option<(i32, i32)>) -> String {
    match view {
        Some((x, z)) => format!("{ACCEPTED}{x} {z}"),
        None => format!("{ACCEPTED}off"),
    }
}

/// The centre an accepted reply names, as Some(Some(chunk)) or Some(None); None for any other line.
fn read_reply(line: &str) -> Option<Option<(i32, i32)>> {
    parse(&format!("view {}", line.strip_prefix(ACCEPTED)?))
}

#[mod_api::horizon_mod]
impl Hooks for Fixture {
    fn server_chat(&self, host: &mut dyn ServerHost, event: &mut ServerChatEvent<'_>) -> Flow {
        let Some(view) = parse(event.raw_text) else {
            return Flow::Continue;
        };
        let Some(player) = host.player(event.player) else {
            return Flow::Continue;
        };
        let set = player.set_view_center(host, view.map(|(x, z)| ChunkPos::new(x, z)));
        // A refused call changed nothing on the server, so its reply leaves the camera alone.
        let reply = if set {
            accepted(view)
        } else {
            format!("view centre refused: {view:?}")
        };
        event.replies.push(Text::literal(reply));
        Flow::Cancel
    }

    fn client_chat_received(
        &self,
        _: &mut dyn ClientHost,
        event: &mut ClientChatReceivedEvent<'_>,
    ) -> Flow {
        if event.system
            && let TextContent::Literal(line) = &event.message.content
            && let Some(view) = read_reply(line)
        {
            *CAMERA.lock().unwrap() = view;
        }
        Flow::Continue
    }

    fn camera_pose(&self, _: &dyn ClientHost, event: &mut CameraPoseEvent<'_>) {
        if let Some((x, z)) = *CAMERA.lock().unwrap() {
            // Above the chunk, looking down on it and the ones around it.
            *event.position = [f64::from(x * 16 + 8), 160.0, f64::from(z * 16 - 40)];
            event.pitch = 50.0;
            event.yaw = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{accepted, parse, read_reply};

    #[test]
    fn reads_the_chat_commands() {
        assert_eq!(parse("view 40 -12"), Some(Some((40, -12))));
        assert_eq!(parse("view off"), Some(None));
        assert_eq!(parse("view 40"), None);
        assert_eq!(parse("view 40 -12 7"), None);
        assert_eq!(parse("hello"), None);
    }

    #[test]
    fn only_an_accepted_reply_moves_the_camera() {
        assert_eq!(
            read_reply(&accepted(Some((40, -12)))),
            Some(Some((40, -12)))
        );
        assert_eq!(read_reply(&accepted(None)), Some(None));
        assert_eq!(read_reply("view centre refused: Some((1875001, 0))"), None);
        assert_eq!(read_reply("someone said view centre 1 2"), None);
    }
}
