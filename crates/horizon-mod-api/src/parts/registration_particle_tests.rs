#[cfg(test)]
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn particle_load_refuses_atomically_and_publishes_provider_indices() {
        let registrar = crate::parts::PartRegistrar::new("test");
        registrar
            .registry_keys
            .borrow_mut()
            .particles
            .request("test", "test:spark")
            .unwrap();
        crate::parts::install_registrar(registrar, &crate::__private::part_tests::rule_schema())
            .unwrap();
        let id = crate::Identifier::parse_namespaced("test:spark").unwrap();
        assert!(
            resolve_particles(["minecraft:smoke"], |_| Some(ParticleKind::Vanilla(7))).is_err()
        );
        assert!(particle_key(&id).is_none());
        let calls = std::cell::Cell::new(0);
        resolve_particles(["minecraft:smoke", "test:spark"], |name| {
            calls.set(calls.get() + 1);
            Some(if name == "test:spark" {
                ParticleKind::Custom
            } else {
                ParticleKind::Vanilla(7)
            })
        })
        .unwrap();
        let key = particle_key(&id).unwrap();
        assert_eq!(particle_name(key), Some("test:spark"));
        assert_eq!(particle_kind(key), Some(ParticleKind::Custom));
        let smoke =
            particle_key(&crate::Identifier::parse_namespaced("minecraft:smoke").unwrap()).unwrap();
        assert_eq!(particle_kind(smoke), Some(ParticleKind::Vanilla(7)));
        assert_eq!(calls.get(), 2);
        assert_eq!(
            mod_vocab::__private::parts_installation::validate_particle_key(
                mod_vocab::__private::parts_installation::particle_key_slot(key)
            ),
            Ok(key)
        );
        assert!(mod_vocab::__private::parts_installation::validate_particle_key(u32::MAX).is_err());
        assert!(resolve_particles(["test:spark"], |_| Some(ParticleKind::Custom)).is_err());
        assert_eq!(particle_kind(smoke), Some(ParticleKind::Vanilla(7)));
        assert_eq!(calls.get(), 2);
    }
}
