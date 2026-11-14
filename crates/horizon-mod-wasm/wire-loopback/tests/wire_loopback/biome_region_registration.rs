/// The region the guest declares: fractional and negative climate bounds, so the host reads
/// every float back bit for bit.
pub(super) fn region() -> mod_api::BiomeRegion {
    mod_api::BiomeRegion {
        dimension: "minecraft:nether".into(),
        weight: 7,
        points: vec![
            mod_api::BiomePoint {
                biome: "wire_test:ember_marsh".into(),
                temperature: (-0.375, 1.25),
                humidity: (-2.0, 2.0),
                continentalness: (0.0, 0.0),
                erosion: (-0.1, 0.1),
                depth: (0.0, 1.0),
                weirdness: (0.3, 0.3),
                offset: 0.125,
            },
            mod_api::BiomePoint {
                biome: "minecraft:basalt_deltas".into(),
                temperature: (0.0, 0.0),
                humidity: (0.0, 0.0),
                continentalness: (0.0, 0.0),
                erosion: (0.0, 0.0),
                depth: (0.0, 0.0),
                weirdness: (0.0, 0.0),
                offset: 1.0,
            },
        ],
    }
}

pub(super) fn register(registrar: &mut dyn mod_api::Registrar) {
    registrar
        .declare_biome_region("ember_marsh", region())
        .unwrap();
}

pub(super) struct BiomeRegionRegistrationHooks;
impl BiomeRegionRegistrationHooks {
    pub(super) fn register(
        &self,
        registrar: &mut dyn mod_api::Registrar,
        cx: &mod_api::RegistrationContext,
    ) {
        self::register(registrar);
    }
}
