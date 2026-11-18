//! Tests for the particle conversions, which live in horizon-gameplay.
use super::{TryIntoApi, TryIntoEngine};
use horizon_client::particles::{ParticleOptions as EngineOptions, ParticleType};
use horizon_gameplay::api_values::particle_values::ParticleValueError;
use mod_api::__private::ParticleKind;
#[cfg(test)]
mod tests {
    use super::*;
    fn prepare() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        mod_api::__private::resolve_particles(
            ["minecraft:smoke", "minecraft:trail", "test:spark"],
            |id| {
                if id == "test:spark" {
                    Some(ParticleKind::Custom)
                } else {
                    ParticleType::from_name(id).map(|ty| ParticleKind::Vanilla(ty as u32))
                }
            },
        )
        .unwrap();
    }
    #[test]
    fn particle_keys_preserve_provider_and_every_option_slot() {
        prepare();
        let mut original = EngineOptions::trail([3.0, 4.0, 5.0], 0x112233, 19);
        original.legacy_entity_effect = Some(true);
        original.legacy_trail_lifetime = true;
        original.scale = 2.5;
        original.to_color = [0.1, 0.3, 0.7];
        original.block = Some(("test:sprite".into(), 0x123456));
        original.item = Some(vec!["test:layer".into()]);
        original.vibration = Some((
            mod_api::client::particles::PositionSource::Entity {
                id: -7,
                y_offset: 0.5,
            },
            [1.0, 2.0, 3.0],
            11,
        ));
        let api = original.try_into_api().unwrap();
        assert_eq!(
            mod_api::__private::particle_kind(api.ty),
            Some(ParticleKind::Vanilla(ParticleType::Trail as u32))
        );
        assert_eq!(api.legacy_entity_effect, Some(true));
        assert!(api.legacy_trail_lifetime);
        let result = api.try_into_engine(&()).unwrap();
        assert_eq!(result.legacy_entity_effect, Some(true));
        assert!(result.legacy_trail_lifetime);
        assert_eq!(result.ty, ParticleType::Trail);
        assert_eq!(result.trail, Some(([3.0, 4.0, 5.0], 19)));
        assert_eq!(
            result.color,
            [17.0 / 255.0, 34.0 / 255.0, 51.0 / 255.0, 1.0]
        );
        assert_eq!(result.scale, 2.5);
        assert_eq!(result.to_color, [0.1, 0.3, 0.7]);
        assert_eq!(result.block, Some(("test:sprite".into(), 0x123456)));
        assert_eq!(result.item, Some(vec!["test:layer".into()]));
        assert_eq!(
            result.vibration,
            Some((
                mod_api::client::particles::PositionSource::Entity {
                    id: -7,
                    y_offset: 0.5
                },
                [1.0, 2.0, 3.0],
                11
            ))
        );
    }
    #[test]
    fn particle_geyser_options_cross_the_boundary() {
        prepare();
        let value = EngineOptions::geyser_base(ParticleType::Smoke, 7, 1.25)
            .try_into_api()
            .unwrap();
        assert_eq!(value.try_into_engine(&()).unwrap().geyser, Some((7, 1.25)));
    }
    #[test]
    fn particle_custom_provider_borrows_its_retained_name() {
        prepare();
        let value = EngineOptions::named("test:spark").try_into_api().unwrap();
        let published = mod_api::__private::particle_name(value.ty).unwrap();
        let output = value.try_into_engine(&()).unwrap();
        assert_eq!(output.mod_particle.as_deref(), Some("test:spark"));
        assert!(std::ptr::eq(
            output.mod_particle.as_deref().unwrap(),
            published
        ));
    }
    #[test]
    fn particle_boundary_refuses_nonfinite_nested_input() {
        prepare();
        let mut options = EngineOptions::simple(ParticleType::Smoke)
            .try_into_api()
            .unwrap();
        options.vibration = Some((
            mod_api::client::particles::PositionSource::Entity {
                id: 1,
                y_offset: f32::NAN,
            },
            [0.0; 3],
            3,
        ));
        assert_eq!(
            options.try_into_engine(&()),
            Err(ParticleValueError::NonFinite)
        );
    }
}
