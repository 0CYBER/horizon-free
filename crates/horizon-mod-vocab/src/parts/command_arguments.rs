//! Command argument types mods register (`Registrar::register_command_argument`), kept for the
//! command trees built at every world open and refresh.
use super::*;
use std::sync::Arc;

/// A command argument type a mod registered, and the id a `Command` usage names.
#[derive(Clone)]
pub(crate) struct CommandArgumentInfo {
    pub(crate) id: Box<str>,
    pub(crate) argument: Arc<dyn crate::CommandArgumentType>,
}

/// The command argument types every installed mod registered, in registration order.
pub(crate) fn installed_command_arguments() -> &'static [CommandArgumentInfo] {
    installed().map_or(&[], |installed| installed.keys.command_arguments.as_slice())
}

impl dyn Registrar + '_ {
    /// Register command argument type `name` (`machine` or `mymod:machine`), once, from
    /// `Hooks::register`: a usage then names it as an argument's kind,
    /// `Command::new("machine use <machine:mymod:machine>")`, and the game parses and
    /// completes that argument with it. A name in an unowned namespace, an empty name, and an
    /// id registered twice refuse the mods at startup, naming the mod.
    pub fn register_command_argument<T: crate::CommandArgumentType>(
        &mut self,
        name: &str,
        argument: T,
    ) {
        self.add_command_argument(name, Arc::new(argument));
    }
}

/// Registrar operations for command argument types.
pub trait CommandArgumentsRegistrar {
    /// Register command argument type `name` with `argument`; mods reach it through
    /// `register_command_argument`.
    fn add_command_argument(&mut self, name: &str, argument: Arc<dyn crate::CommandArgumentType>);
}
