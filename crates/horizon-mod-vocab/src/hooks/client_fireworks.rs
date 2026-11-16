#[doc = r#"a firework's explosions drawn on this client (`create_fireworks`), with or without their sound."#]
pub trait Notes {
    #[notes = r#"`ClientHost::create_fireworks(&FireworksRequest) -> bool` draws a firework's explosions at a point in this client's world through the same starter a bursting rocket uses: each `FireworkExplosion`'s shape, colours, fade colours, trail and twinkle, and the rocket's velocity carried by the sparks. `audible` gates the blast and twinkle sounds, as a rocket's silence does; the sparks and flash draw either way. No explosions puffs a little smoke, as vanilla's. The request is the API's own record, the same on native and wasm. It returns false on servers, without a client player/level, for non-finite input or for more than 256 explosions. Bundles need "hooks": @API:client_fireworks@."#]
    fn client_fireworks();
}
