//! Biome-region registration and admission tests.
use super::*;

impl BiomeRegionsRegistrar for PartRegistrar {
    fn declare_biome_region(
        &mut self,
        path: &str,
        region: crate::BiomeRegion,
    ) -> Result<(), crate::BiomeRegionError> {
        let result = (|| {
            let id = self
                .owned_declaration_id(path)
                .map_err(|_| crate::BiomeRegionError::InvalidId)?;
            if self.biome_regions.iter().any(|d| d.id == id) {
                return Err(crate::BiomeRegionError::Duplicate);
            }
            region.check()?;
            self.biome_regions.push(BiomeRegionDeclaration {
                mod_id: self.mod_id.to_owned(),
                id,
                region,
            });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} biome region {path:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::*;

    #[cfg(test)]
    fn region(weight: u32) -> BiomeRegion {
        BiomeRegion {
            dimension: "minecraft:overworld".into(),
            weight,
            points: vec![BiomePoint {
                biome: "minecraft:plains".into(),
                temperature: (0.0, 0.0),
                humidity: (0.0, 0.0),
                continentalness: (0.0, 0.0),
                erosion: (0.0, 0.0),
                depth: (0.0, 0.0),
                weirdness: (0.0, 0.0),
                offset: 0.0,
            }],
        }
    }

    #[test]
    pub(crate) fn biome_region_declarations_validate_and_refuse_duplicates() {
        let mut registrar = super::PartRegistrar::new("ruby");
        registrar
            .declare_biome_region("highlands", region(5))
            .unwrap();
        // The same id twice is a duplicate, spelled bare or with the namespace.
        assert_eq!(
            registrar.declare_biome_region("ruby:highlands", region(5)),
            Err(BiomeRegionError::Duplicate)
        );
        // The id stays in the declaring mod's namespace and is well formed.
        assert_eq!(
            registrar.declare_biome_region("foreign:highlands", region(5)),
            Err(BiomeRegionError::InvalidId)
        );
        assert_eq!(
            registrar.declare_biome_region("High Lands", region(5)),
            Err(BiomeRegionError::InvalidId)
        );
        // The region itself is checked.
        assert_eq!(
            registrar.declare_biome_region("light", region(0)),
            Err(BiomeRegionError::InvalidWeight)
        );
        // The refusals name the mod; the good declaration stands.
        assert_eq!(registrar.biome_regions.len(), 1);
        assert_eq!(registrar.refused.len(), 4);
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod ruby"))
        );
    }

    /// Mods in either dispatch order install the same list, sorted by mod id, then region id.
    #[test]
    pub(crate) fn declared_regions_install_in_mod_then_id_order() {
        struct Regions(&'static [(&'static str, u32)]);
        #[mod_api::__subscriptions]
        impl Hooks for Regions {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                for &(path, weight) in self.0 {
                    r.declare_biome_region(path, region(weight)).unwrap();
                }
            }
        }
        let zeta: &'static [(&'static str, u32)] = &[("plateau", 1), ("canyon", 2)];
        let alpha: &'static [(&'static str, u32)] = &[("marsh", 3)];
        let expected = [
            ("alpha", "alpha:marsh", 3),
            ("zeta", "zeta:canyon", 2),
            ("zeta", "zeta:plateau", 1),
        ];
        for order in [
            [("zeta", zeta), ("alpha", alpha)],
            [("alpha", alpha), ("zeta", zeta)],
        ] {
            let mods = order.map(|(id, regions)| ModEntry::new(id, Box::new(Regions(regions))));
            crate::parts::tests::install_parts_fixture(&mods).unwrap();
            let installed: Vec<_> = crate::__private::biome_region_declarations()
                .iter()
                .map(|d| (d.mod_id.as_str(), d.id.as_str(), d.region.weight))
                .collect();
            assert_eq!(installed, expected);
        }
    }
}
