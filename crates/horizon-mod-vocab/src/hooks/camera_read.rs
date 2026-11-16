pub trait Notes {
    #[notes = r#"## Camera entity

`ClientHost::camera_entity()` returns an `EntityRef` for the entity the view follows
(`Minecraft.getCameraEntity`): the local player, unless a server attached the camera to another
entity. It is `None` on the title screen and outside a world. Together with `camera()` and
`camera_type()` it lets a minimap or freecam mod tell its own view from a spectated one.

`ClientHost::set_camera_entity(entity)` is `Minecraft.setCameraEntity`: it moves the view onto a
mob (or back onto the player with the player's own ref) and returns whether it did. It selects
the entity only; the camera angle stays the entity's, and `camera_pose` is the way to override it.

## Client heights

`ClientHost::client_surface_heights(columns, heightmap)` is `LevelReader.getHeight(type, x, z)` on a
client level: each column's height from the chunks the server has sent this window, `None` for a
column it holds no chunk of. A minimap or a tracker reads many columns in one call instead of
scanning blocks. It is the client's own view, so it follows the server's block changes as they arrive."#]
    fn camera_read_note0();
}
