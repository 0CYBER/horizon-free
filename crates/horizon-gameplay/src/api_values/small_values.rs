//! Small API-owned recipe and input values.
use super::{TryIntoApi, TryIntoEngine};
use std::convert::Infallible;

impl TryIntoApi for horizon_items::firework::FireworkShape {
    type Api = mod_api::FireworkShape;
    type Error = Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        use horizon_items::firework::FireworkShape as E;
        use mod_api::FireworkShape as A;
        Ok(match self {
            E::SmallBall => A::SmallBall,
            E::LargeBall => A::LargeBall,
            E::Star => A::Star,
            E::Creeper => A::Creeper,
            E::Burst => A::Burst,
        })
    }
}
impl TryIntoEngine<()> for mod_api::FireworkShape {
    type Engine = horizon_items::firework::FireworkShape;
    type Error = Infallible;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        use horizon_items::firework::FireworkShape as E;
        use mod_api::FireworkShape as A;
        Ok(match self {
            A::SmallBall => E::SmallBall,
            A::LargeBall => E::LargeBall,
            A::Star => E::Star,
            A::Creeper => E::Creeper,
            A::Burst => E::Burst,
            _ => unreachable!("all API firework shapes are handled"),
        })
    }
}
impl TryIntoEngine<()> for mod_api::KeyConflictContext {
    type Engine = horizon_mod_package::mod_list::KeyConflictContext;
    type Error = Infallible;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        use horizon_mod_package::mod_list::KeyConflictContext as E;
        use mod_api::KeyConflictContext as A;
        Ok(match self {
            A::Universal => E::Universal,
            A::InGame => E::InGame,
            A::Gui => E::Gui,
            _ => unreachable!("all API conflict contexts are handled"),
        })
    }
}

impl TryIntoApi for horizon_mod_package::manifest::JarInputsError {
    type Api = mod_api::JarInputsError;
    type Error = Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        use horizon_mod_package::manifest::JarInputsError as E;
        use mod_api::JarInputsError as A;
        Ok(match self {
            E::Missing(m) => A::Missing(Box::new(mod_api::MissingJar::new(
                m.owners.clone(),
                m.jar.id().into(),
                m.jar.version().into(),
                m.jar.sha512().into(),
                m.path.display().to_string(),
            ))),
            E::Io { path, source } => A::Io {
                path: path.display().to_string(),
                message: source.to_string(),
            },
            E::Invalid { path, source } => A::Invalid {
                path: path.display().to_string(),
                message: source.to_string(),
            },
            _ => unreachable!("all original-jar failures are handled"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_value_conversion_cases() {
        use horizon_items::firework::FireworkShape as F;
        for (index, shape) in [F::SmallBall, F::LargeBall, F::Star, F::Creeper, F::Burst]
            .iter()
            .enumerate()
        {
            let api = shape.try_into_api().unwrap();
            assert_eq!(api as usize, index);
            assert_eq!(api.try_into_engine(&()).unwrap(), *shape);
        }
        use mod_api::KeyConflictContext as K;
        for (index, context) in [K::Universal, K::InGame, K::Gui].iter().enumerate() {
            assert_eq!(context.try_into_engine(&()).unwrap() as usize, index);
        }
    }
}

#[cfg(test)]
mod jar_tests {
    use super::*;
    #[test]
    fn original_jar_diagnostics_preserve_identity_and_failure_kind() {
        use horizon_mod_package::manifest::{JarInputsError as E, JarRef, MissingJar};
        let jar = JarRef::new("original".into(), "1".into(), "a".repeat(128)).unwrap();
        let missing = E::Missing(Box::new(MissingJar {
            owners: vec!["first".into(), "second".into()],
            jar,
            path: "cache/exact.jar".into(),
        }));
        let mod_api::JarInputsError::Missing(value) = missing.try_into_api().unwrap() else {
            panic!("wrong missing case")
        };
        assert_eq!(value.owners, ["first", "second"]);
        assert_eq!(value.id, "original");
        assert_eq!(value.version, "1");
        assert_eq!(value.sha512, "a".repeat(128));
        assert_eq!(value.path, "cache/exact.jar");
        let io = E::Io {
            path: "inputs".into(),
            source: std::io::Error::other("disk failure"),
        };
        assert!(
            matches!(io.try_into_api().unwrap(), mod_api::JarInputsError::Io { path, message } if path == "inputs" && message == "disk failure")
        );
        let invalid = E::Invalid {
            path: "record".into(),
            source: serde_json::from_str::<serde_json::Value>("{").unwrap_err(),
        };
        assert!(
            matches!(invalid.try_into_api().unwrap(), mod_api::JarInputsError::Invalid { path, message } if path == "record" && !message.is_empty())
        );
    }
}
