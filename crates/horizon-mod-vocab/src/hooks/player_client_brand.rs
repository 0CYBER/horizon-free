pub trait Notes {
    #[notes = r#"## Client brand

`PlayerRef::client_brand(host)` answers which client a player joined with, from the brand string
the connection recorded (`ServerPlayer.clientBrand`). Horizon's own clients say `horizon`, which
is what the game itself checks before it sends them Horizon-only packets, so a mod that does the
same asks here rather than guessing from the name."#]
    fn player_client_brand_note0();
}
