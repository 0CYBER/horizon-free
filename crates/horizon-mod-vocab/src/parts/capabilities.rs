use super::*;
pub(crate) type Capabilities<P> = HashMap<&'static str, HashMap<Box<str>, Vec<CapabilityPart<P>>>>;

/// A capability provider a mod registered: a [`BlockProviderOf`], [`ItemProviderOf`] or
/// [`EntityProviderOf`].
#[derive(Clone)]
pub(crate) struct CapabilityPart<P> {
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    /// The handler type it gives, for refusing a capability name answered with two.
    pub(crate) type_name: Arc<str>,
    /// Asked ahead of the game's own provider.
    pub(crate) before_game: bool,
    /// Its place among every mod's registrations, which orders a block's providers and those of
    /// the block entity type standing there as one list.
    pub(crate) order: usize,
    pub(crate) provider: P,
}

/// A capability provider whose handler type is a mod's own (a key a library's `[api]` crate
/// publishes), erased: only mods linked with that crate know its handler type.
// hidden: the registrar's erased provider, not part of the mod surface
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: transfers the handler name and a retained provider resource id, reporting unsupported handlers rather than serializing the native Any object.
#[doc(hidden)]
#[derive(Clone)]
pub struct OwnProvider {
    /// The `Arc` of the provider trait object, which a lookup of that handler type downcasts:
    /// a wasm mod's, in the game or another group, is the game's proxy, which no lookup takes.
    pub(crate) provider: Arc<dyn Any + Send + Sync>,
    pub(crate) handler: Arc<str>,
}

/// The registrar's table, as the game reads it.
pub(crate) struct InstalledProviders;

pub(crate) fn next<P: Clone>(
    table: &Capabilities<P>,
    name: &str,
    id: &str,
    index: &mut u32,
) -> Option<(P, &'static str, bool)> {
    next_provider(table.get(name)?.get(id)?, index)
}

/// The most providers one capability may have for one block or one block entity type: a
/// position's lookup keeps its place in both lists in the one `u32` cursor, 16 bits each.
pub(crate) const MAX_PROVIDERS_AT: usize = u16::MAX as usize;

/// The next enabled provider of two lists asked as one, `first` and `second` (a block entity
/// type's and a block's): those asked ahead of the game, then the others, each group in
/// registration order across both lists. `index` holds the place in each list, `first`'s in
/// its low 16 bits and `second`'s in its high ones, so each step looks at the two lists' next
/// provider and takes one, past disabled ones, as [`next_provider`] does for one list.
pub(crate) fn next_merged<P: Clone>(
    first: &[CapabilityPart<P>],
    second: &[CapabilityPart<P>],
    index: &mut u32,
) -> Option<(P, &'static str, bool)> {
    // Each list is already in this order: those ahead of the game first, by registration.
    let rank = |part: &CapabilityPart<P>| (!part.before_game, part.order);
    let (mut at_first, mut at_second) = (*index & 0xffff, *index >> 16);
    loop {
        let part = match (first.get(at_first as usize), second.get(at_second as usize)) {
            (Some(a), Some(b)) if rank(b) < rank(a) => {
                at_second += 1;
                b
            }
            (Some(a), _) => {
                at_first += 1;
                a
            }
            (None, Some(b)) => {
                at_second += 1;
                b
            }
            (None, None) => return None,
        };
        *index = at_first | at_second << 16;
        if crate::mod_index_on(part.mod_index) {
            return Some((part.provider.clone(), part.mod_id, part.before_game));
        }
    }
}

/// A provider by its handler type: one case for each handler type `mod_api`'s keys name
/// (generated from them), and [`OwnProvider`] for a mod's own.
#[allow(missing_docs)]
pub(crate) mod providers_of {
    use super::*;
    // The keys' handler types and contexts, as the generated cases name them.
    #[allow(unused_imports)]
    use crate::*;
    include!(concat!(
        env!("OUT_DIR"),
        "/capability_providers_generated.rs"
    ));
}

/// Who answers a capability: a mod's provider, the game's own, or nobody.
pub(crate) enum Answer<T: ?Sized> {
    Mod(Capability<T>),
    Game,
    Nobody,
}

/// The handler `answer` names: a mod's, or `game`, the game's own when it answered.
pub(crate) fn answered<T: ?Sized>(
    answer: Answer<T>,
    game: Option<Box<T>>,
) -> Option<Capability<T>> {
    match answer {
        Answer::Mod(found) => Some(found),
        Answer::Game => game.map(|handler| Capability {
            handler,
            owner: None,
        }),
        Answer::Nobody => None,
    }
}

/// Asks the providers `next` gives (advancing a registration cursor in dispatch order: those
/// registered `before_game`, then the others), and the game's own (`game`: whether it answers) between the
/// two, in the fork's chosen order (NeoForge's javadoc doesn't state one between mods'
/// providers and the game's); the first to answer wins. `get` takes a provider of this
/// capability's handler type out of its case: a provider of a mod's own type the asker isn't
/// linked with is passed over.
pub(crate) fn ask_providers<O, P: ?Sized + 'static, T: ?Sized>(
    host: &mut dyn ServerHost,
    next: impl Fn(&mut u32) -> Option<(O, &'static str, bool)>,
    get: impl Fn(&O) -> Option<&Arc<P>>,
    call: impl Fn(&P, &mut dyn ServerHost) -> Option<Box<T>>,
    game: impl FnOnce(&mut dyn ServerHost) -> bool,
) -> Answer<T> {
    let mut game = Some(game);
    let mut index = 0;
    while let Some((provider, owner, before_game)) = next(&mut index) {
        if !before_game
            && let Some(game) = game.take()
            && game(host)
        {
            return Answer::Game;
        }
        if let Some(handler) = get(&provider).and_then(|provider| call(provider, host)) {
            return Answer::Mod(Capability {
                handler,
                owner: Some(owner),
            });
        }
    }
    match game.is_some_and(|game| game(host)) {
        true => Answer::Game,
        false => Answer::Nobody,
    }
}

/// CapabilityProviders operations for capabilities.
// hidden: per-kind lookup implementation, exposed for generated provider bindings
#[doc(hidden)]
pub trait CapabilitiesCapabilityProviders {
    /// Whether any mod registered capability `name` for the things `by` names. This presence
    /// check includes disabled mods; enumeration skips their providers.
    fn provided(&self, by: ProvidedBy, name: &str) -> bool;
    /// Known block capability names in declaration/discovery order.
    fn block_capability_names(&self) -> Vec<String>;
    /// Whether this capability was explicitly marked safe to forward.
    fn block_capability_proxyable(&self, name: &str) -> bool;
    /// Known block capability names explicitly marked safe to forward, in discovery order.
    fn block_capability_proxyable_names(&self) -> Vec<String>;
    /// How many times a mod was switched on or off, as the game counts it: a kept capability
    /// compares it to tell that a provider may have come or gone (`BlockCapabilityCache`).
    fn mods_switched(&self) -> u64;
}

impl CapabilitiesCapabilityProviders for InstalledProviders {
    fn provided(&self, by: ProvidedBy, name: &str) -> bool {
        installed().is_some_and(|installed| match by {
            ProvidedBy::BlockEntity => installed.keys.capabilities.contains_key(name),
            ProvidedBy::Block => installed.keys.block_capabilities.contains_key(name),
            ProvidedBy::Item => installed.keys.item_capabilities.contains_key(name),
            ProvidedBy::Entity => installed.keys.entity_capabilities.contains_key(name),
        })
    }
    fn block_capability_names(&self) -> Vec<String> {
        match installed() {
            Some(installed) => installed
                .keys
                .block_capability_order
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            None => providers_of::builtin_block_capabilities()
                .into_iter()
                .map(|(name, _)| name.to_owned())
                .collect(),
        }
    }
    fn block_capability_proxyable(&self, name: &str) -> bool {
        installed()
            .and_then(|installed| installed.keys.block_capability_metadata.get(name))
            .is_some_and(|(_, proxyable)| *proxyable == Some(true))
    }
    fn block_capability_proxyable_names(&self) -> Vec<String> {
        let Some(installed) = installed() else {
            return Vec::new();
        };
        installed
            .keys
            .block_capability_order
            .iter()
            .filter(|name| self.block_capability_proxyable(name))
            .map(|name| (*name).to_owned())
            .collect()
    }
    fn mods_switched(&self) -> u64 {
        crate::runtime::mods_switched()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    struct Empty;
    #[mod_api::__subscriptions]
    impl Hooks for Empty {}

    #[test]
    fn merged_cursor_exhausts_both_halves_without_wrapping_past_disabled_entries() {
        install(
            vec![
                ModEntry::new("enabled", Box::new(Empty)),
                ModEntry::new("disabled", Box::new(Empty)),
            ]
            .into(),
        );
        assert!(set_mod_enabled("disabled", false));
        let providers: Vec<_> = (0..65_535)
            .map(|order| CapabilityPart {
                mod_id: if order == 65_533 {
                    "disabled"
                } else {
                    "enabled"
                },
                mod_index: if order == 65_533 { 1 } else { 0 },
                type_name: "test:handler".into(),
                before_game: false,
                order,
                provider: order,
            })
            .collect();
        for high_half in [false, true] {
            let (first, second) = if high_half {
                (&[][..], providers.as_slice())
            } else {
                (providers.as_slice(), &[][..])
            };
            let mut cursor = 0;
            let mut count = 0;
            let mut last = None;
            while let Some((provider, owner, before_game)) = next_merged(first, second, &mut cursor)
            {
                assert_eq!(owner, "enabled");
                assert!(!before_game);
                count += 1;
                last = Some(provider);
            }
            assert_eq!(count, 65_534);
            assert_eq!(last, Some(65_534));
            assert_eq!(cursor, if high_half { 0xffff_0000 } else { 0x0000_ffff });
            assert_eq!(next_merged(first, second, &mut cursor), None);
            assert_eq!(cursor, if high_half { 0xffff_0000 } else { 0x0000_ffff });
        }
    }
}
