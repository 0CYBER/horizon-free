//! Map image delivery through a validated player handle.

pub trait Notes {
    #[notes = r#"## Sending a map image

`PlayerRef::send_map` sends a complete 128-by-128 map image through a
`&mut dyn ServerHost`. It validates the live player handle and refuses fake
players; a connectionless actor never produces a protocol packet. The image
uses map colour bytes and is not saved in the world's map storage. Send it
again after a player reconnects if no saved map owns that id."#]
    fn player_send_map();
}
