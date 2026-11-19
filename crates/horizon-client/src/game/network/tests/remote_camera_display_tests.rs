//! `ClientboundSetCameraPacket` naming a display entity, a server cutscene's
//! usual camera (`ClientPacketListener.handleSetCamera` takes any entity
//! `Level.getEntity` finds): the view moves to the display, so the player
//! stops being the controlled camera (`LocalPlayer.isControlledCamera`) and
//! neither walks, flies nor turns the view.
use super::{CameraEntities, camera_after_packet};
use crate::game::text_displays::{RemoteTextDisplay, Shows};
use std::collections::HashMap;

#[test]
fn a_server_camera_on_a_display_takes_the_view_from_the_player() {
    let displays = HashMap::from([(
        21,
        RemoteTextDisplay::new(Shows::Item, [3.5, 70.0, -8.25], 135.0),
    )]);
    let entities = CameraEntities {
        mobs: &[],
        stands: &[],
        boats: &[],
        frames: &[],
        tnts: &[],
        displays: &displays,
    };

    // The packet names the display: the camera leaves the player (id 5).
    let camera = camera_after_packet(None, 5, 21, entities.holds(21));
    assert_eq!(camera, Some(21));

    // `Camera.alignWithEntity`: the display's position and yaw, its eyes at
    // its feet (`EntityType.ITEM_DISPLAY` is `sized(0.0F, 0.0F)`), not living.
    let view = entities.view(21).expect("a display is a camera");
    assert_eq!(view.position, [3.5, 70.0, -8.25]);
    assert_eq!((view.y_rot, view.x_rot), (135.0, 0.0));
    assert_eq!(view.eye_height, 0.0);
    assert!(view.living.is_none());
}
