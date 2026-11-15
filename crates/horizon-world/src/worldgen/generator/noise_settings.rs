//! Shared NoiseGeneratorSettings acceptance for terrain construction and registry reads.
use super::*;

pub(super) struct DecodedNoiseSettings {
    pub(super) settings: Settings,
    pub(super) router: RouterSamplers,
    pub(super) debug_density: Vec<(String, Arc<dyn DensitySampler>)>,
    pub(super) aquifer: Option<[Arc<dyn DensitySampler>; 6]>,
    pub(super) rule: MaterialRule,
    pub(super) spawn_target: Vec<SpawnTargetPoint>,
}

/// The 32-bit heights of a noise-settings definition, inside its level's build height.
#[derive(Debug, PartialEq)]
struct Heights {
    min_y: i32,
    height: i32,
    level_min_y: i32,
    level_height: i32,
    sea_level: i32,
}

/// Reads the heights of `settings_json`, refusing a value or a range past 32 bits, a negative
/// height and a noise range outside `level` (the noise range itself when `level` is `None`).
fn heights(settings_json: &Value, level: Option<(i32, i32)>) -> Result<Heights, GeneratorError> {
    let int = |value: &Value, what: &str| {
        value
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| resource(what))
    };
    let noise = &settings_json["noise"];
    let min_y = int(&noise["min_y"], "noise min_y")?;
    let height = int(&noise["height"], "noise height")?;
    let (level_min_y, level_height) = level.unwrap_or((min_y, height));
    let (Some(end), Some(level_end)) = (
        min_y.checked_add(height),
        level_min_y.checked_add(level_height),
    ) else {
        return Err(resource("noise range beyond 32-bit heights"));
    };
    if height < 0 || min_y < level_min_y || end > level_end {
        return Err(resource("noise range outside the dimension's build height"));
    }
    Ok(Heights {
        min_y,
        height,
        level_min_y,
        level_height,
        sea_level: int(&settings_json["sea_level"], "sea_level")?,
    })
}

impl GeneratorData {
    pub(super) fn decode_noise_settings(
        &self,
        settings_json: &Value,
        seed: i64,
        level: Option<(i32, i32)>,
    ) -> Result<DecodedNoiseSettings, GeneratorError> {
        let data = self;
        let registry = data.registry;
        let heights = heights(settings_json, level)?;
        let codec = crate::registry::BlockStateJsonCodec::new(registry, data.integers.clone())
            .map_err(|e| resource(e.to_string()))?;
        let settings = Settings {
            noise_min_y: heights.min_y,
            noise_height: heights.height,
            level_min_y: heights.level_min_y,
            level_height: heights.level_height,
            sea_level: heights.sea_level,
            default_block: state(&codec, &settings_json["default_block"])?,
            default_fluid: state(&codec, &settings_json["default_fluid"])?,
            legacy: settings_json["legacy_random_source"]
                .as_bool()
                .unwrap_or(false),
            disable_mob_generation: settings_json["disable_mob_generation"]
                .as_bool()
                .unwrap_or(false),
        };
        let mut densities = DensityRegistry::from_json(&data.noises, &data.densities)?;
        let router =
            NoiseRouter::from_json(&settings_json["noise_router"].to_string(), &mut densities)?;
        // NoiseGeneratorSettings.debugFunctions: a registered density or an
        // inline one, each with its label.
        let debug_functions = settings_json
            .get("debug_functions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|entry| {
                let label = entry["label"]
                    .as_str()
                    .ok_or_else(|| resource("debug label"))?;
                let id = match &entry["function"] {
                    Value::String(name) => densities
                        .reference(name)
                        .ok_or_else(|| resource(format!("debug function {name}")))?,
                    inline => densities.decode_functions(&[&inline.to_string()])?[0],
                };
                Ok((label.to_owned(), id))
            })
            .collect::<Result<Vec<_>, GeneratorError>>()?;
        let aquifers = settings_json
            .get("aquifers")
            .filter(|v| !v.is_null())
            .map(|v| AquiferConfig::from_json(&v.to_string(), &mut densities))
            .transpose()?;
        let mut library = MaterialRuleLibrary::from_json(
            &data.rules,
            &data.conditions,
            &codec,
            densities,
            seed,
            settings.legacy,
        )?;
        let router = library.compile_router(router)?;
        let debug_density = debug_functions
            .into_iter()
            .map(|(label, id)| Ok((label, library.function_sampler(id)?)))
            .collect::<Result<Vec<_>, GeneratorError>>()?;
        let aquifer = aquifers
            .map(|config| library.compile_aquifer(config))
            .transpose()?
            .map(|samplers| AquiferField::ALL.map(|field| samplers.get(field).clone()));
        let rule_name = settings_json["material_rule"]
            .as_str()
            .ok_or_else(|| resource("material_rule"))?;
        let rule = library.rule(rule_name)?;
        let spawn_target = match settings_json.get("spawn_target") {
            Some(Value::Array(points)) => points
                .iter()
                .map(|point| {
                    let Value::Object(fields) = point else {
                        return Err(resource("spawn_target point"));
                    };
                    let parameters = fields
                        .iter()
                        .map(|(name, interval)| {
                            Ok((
                                library.named_density(name)?,
                                decode_parameter(&interval.to_string())?,
                            ))
                        })
                        .collect::<Result<_, GeneratorError>>()?;
                    Ok(SpawnTargetPoint { parameters })
                })
                .collect::<Result<_, GeneratorError>>()?,
            _ => Vec::new(),
        };
        Ok(DecodedNoiseSettings {
            settings,
            router,
            debug_density,
            aquifer,
            rule,
            spawn_target,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn read(min_y: i64, height: i64, sea_level: i64) -> Result<Heights, String> {
        let settings = json!({"noise": {"min_y": min_y, "height": height}, "sea_level": sea_level});
        heights(&settings, None).map_err(|error| error.to_string())
    }

    #[test]
    fn heights_stop_at_the_32_bit_boundary() {
        let top = i64::from(i32::MAX);
        assert_eq!(
            read(top - 1, 1, top),
            Ok(Heights {
                min_y: i32::MAX - 1,
                height: 1,
                level_min_y: i32::MAX - 1,
                level_height: 1,
                sea_level: i32::MAX,
            })
        );
        assert_eq!(
            read(top, 1, 0).unwrap_err(),
            "invalid generator resources: noise range beyond 32-bit heights"
        );
        assert_eq!(
            read(top + 1, 0, 0).unwrap_err(),
            "invalid generator resources: noise min_y"
        );
        assert_eq!(
            read(0, top + 1, 0).unwrap_err(),
            "invalid generator resources: noise height"
        );
        assert_eq!(
            read(0, 16, top + 1).unwrap_err(),
            "invalid generator resources: sea_level"
        );
        assert_eq!(
            read(0, 16, i64::from(i32::MIN) - 1).unwrap_err(),
            "invalid generator resources: sea_level"
        );
    }

    #[test]
    fn a_negative_height_or_a_range_outside_the_level_is_refused() {
        let outside =
            "invalid generator resources: noise range outside the dimension's build height";
        assert_eq!(read(0, -1, 0).unwrap_err(), outside);
        let settings = json!({"noise": {"min_y": -64, "height": 384}, "sea_level": 63});
        assert!(heights(&settings, Some((-64, 384))).is_ok());
        assert_eq!(
            heights(&settings, Some((-63, 384)))
                .unwrap_err()
                .to_string(),
            outside
        );
        assert_eq!(
            heights(&settings, Some((-64, 383)))
                .unwrap_err()
                .to_string(),
            outside
        );
    }
}
