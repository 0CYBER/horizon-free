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

impl GeneratorData {
    pub(super) fn decode_noise_settings(
        &self,
        settings_json: &Value,
        seed: i64,
        level: Option<(i32, i32)>,
    ) -> Result<DecodedNoiseSettings, GeneratorError> {
        let data = self;
        let registry = data.registry;
        let noise = &settings_json["noise"];
        let min_y = noise["min_y"]
            .as_i64()
            .ok_or_else(|| resource("noise min_y"))? as i32;
        let height = noise["height"]
            .as_i64()
            .ok_or_else(|| resource("noise height"))? as i32;
        let (level_min_y, level_height) = level.unwrap_or((min_y, height));
        if min_y < level_min_y || min_y + height > level_min_y + level_height {
            return Err(resource("noise range outside the dimension's build height"));
        }
        let codec = crate::registry::BlockStateJsonCodec::new(registry, data.integers.clone())
            .map_err(|e| resource(e.to_string()))?;
        let settings = Settings {
            noise_min_y: min_y,
            noise_height: height,
            level_min_y,
            level_height,
            sea_level: settings_json["sea_level"]
                .as_i64()
                .ok_or_else(|| resource("sea_level"))? as i32,
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
