use super::{Hook, HostAccess, PhaseContext, Side};
use syn::Type;

/// The hook's host parameter type and the expression passed by its native method.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct HostParam {
    pub ty: String,
    pub call: &'static str,
}

/// Whether the hook's host is a level host: a `GameLevel` method, or a hook marked
/// `"host": "level"` whose receiver is another level (a hit's `HitLevel`).
pub(super) fn is_level(hook: &Hook) -> bool {
    hook.host == Some(HostAccess::Level) || game_level(hook)
}

fn game_level(hook: &Hook) -> bool {
    hook.target.contains("::GameLevel<")
}

/// Validate catalog access restrictions against the hook's receiver.
pub(super) fn validate(
    hook: &Hook,
    name: &str,
    is_level: bool,
    mutable: Option<bool>,
    ticks: bool,
) -> Result<(), String> {
    if hook.context.is_some() {
        let owns_host = mutable.is_some() || is_level || ticks;
        if owns_host || hook.host.is_some() {
            return Err(format!(
                "{name}: a phase context cannot be combined with a game/level host"
            ));
        }
    }
    if let Some(marker) = hook.host {
        match marker {
            HostAccess::Read if !is_level && hook.side != Side::Client => {}
            // A server hit's level writes through its random host; a both-side one reads, as
            // a both-side `GameLevel` does, and is told its side by `LevelSide`.
            HostAccess::Level
                if !game_level(hook)
                    && !ticks
                    && (hook.side == Side::Server && mutable == Some(true)
                        || hook.side == Side::Both && mutable == Some(false)) => {}
            HostAccess::Read | HostAccess::Level => {
                return Err(format!("{name}: this entry takes no host marker"));
            }
        }
    }
    if !is_level
        && hook.side == Side::Both
        && mutable == Some(false)
        && hook.host != Some(HostAccess::Read)
    {
        return Err(format!(
            "{name}: a both-side hook with a &self target takes host \"read\""
        ));
    }
    if is_level && hook.side == Side::Both && (mutable == Some(true) || ticks) {
        return Err(format!(
            "{name}: a both-side level hook takes the read kind until a client level seam exists"
        ));
    }
    Ok(())
}

/// Remove the native phase input from the event, selecting its public context.
pub(super) fn context_param(
    hook: &Hook,
    args: &mut Vec<(String, &Type)>,
) -> Result<Option<HostParam>, String> {
    let Some(kind) = hook.context else {
        return Ok(None);
    };
    let name = &hook.name;
    let expected = match kind {
        PhaseContext::Registration => "RegistrationContext",
        PhaseContext::DataLoad => "LoadContext",
        PhaseContext::Loop => "LoopContext",
        PhaseContext::Worldgen => "OwningWorldgen",
    };
    let valid = args.first().is_some_and(|(name, ty)| {
        name == "context"
            && matches!(ty, Type::Reference(reference)
            if reference.mutability.is_none() && matches!(&*reference.elem, Type::Path(path)
                if path.path.segments.last().is_some_and(|segment| segment.ident == expected)))
    });
    if !valid {
        return Err(format!(
            "{name}: phase context requires first input context: &{expected}<'_>"
        ));
    }
    args.remove(0);
    Ok(Some(HostParam {
        ty: match kind {
            PhaseContext::Registration => "&RegistrationContext",
            PhaseContext::DataLoad => "&dyn DataLoadContext",
            PhaseContext::Loop => "&dyn LoopContext",
            PhaseContext::Worldgen => "&dyn WorldgenHost",
        }
        .into(),
        call: "context",
    }))
}

/// Select the host once for both generated signatures and the native call.
pub(super) fn host_param(hook: &Hook, mutable: Option<bool>, ticks: bool) -> Option<HostParam> {
    let is_level = is_level(hook);
    mutable.map(|mutable| {
        if !is_level {
            let (ty, call) = match hook.side {
                Side::Server | Side::Both if hook.host == Some(HostAccess::Read) => {
                    ("&dyn Host", "&*self")
                }
                Side::Server => ("&mut dyn ServerHost", "self"),
                Side::Client if mutable => ("&mut dyn ClientHost", "self"),
                Side::Client => ("&dyn ClientHost", "&*self"),
                Side::Both => ("HostSide", "super::execution_role::host_side(self)"),
            };
            return HostParam {
                ty: ty.into(),
                call,
            };
        }
        if hook.side == Side::Both {
            return HostParam {
                ty: "LevelSide".into(),
                call: "super::mod_api_host::level_side(self)",
            };
        }
        let kind = if is_level {
            match (mutable, ticks) {
                (true, true) => "LevelWriteHost",
                (false, true) => "LevelTicksHost",
                (true, false) => "LevelRandomHost",
                (false, false) => "LevelHost",
            }
        } else {
            "Host"
        };
        let mutable = if mutable || (is_level && ticks) {
            "mut "
        } else {
            ""
        };
        HostParam {
            ty: format!("&{mutable}dyn {kind}"),
            call: if ticks {
                "&mut super::mod_api_host::LevelTicks { level: self, ticks }"
            } else {
                "self"
            },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_context_is_not_an_event_field_or_game_host() {
        for (kind, input, public) in [
            (
                PhaseContext::Registration,
                "&RegistrationContext",
                "&RegistrationContext",
            ),
            (
                PhaseContext::DataLoad,
                "&LoadContext<'_>",
                "&dyn DataLoadContext",
            ),
            (PhaseContext::Loop, "&LoopContext", "&dyn LoopContext"),
            (
                PhaseContext::Worldgen,
                "&OwningWorldgen",
                "&dyn WorldgenHost",
            ),
        ] {
            let hook = Hook {
                context: Some(kind),
                ..Hook::sample("horizon_client::game::Game<'_>::native_phase", Side::Both)
            };
            let ty = syn::parse_str(input).unwrap();
            let output = syn::parse_quote!(&mut Vec<String>);
            let mut args = vec![("context".into(), &ty), ("values".into(), &output)];
            assert_eq!(
                context_param(&hook, &mut args).unwrap(),
                Some(HostParam {
                    ty: public.into(),
                    call: "context",
                })
            );
            assert_eq!(args.len(), 1);
            assert_eq!(args[0].0, "values");
            assert!(validate(&hook, "phase", false, None, false).is_ok());
            assert!(validate(&hook, "phase", false, Some(true), false).is_err());
            assert!(context_param(&hook, &mut args).is_err());
            let mutable = syn::parse_str(&input.replacen('&', "&mut ", 1)).unwrap();
            assert!(context_param(&hook, &mut vec![("context".into(), &mutable)]).is_err());
        }
    }
}
