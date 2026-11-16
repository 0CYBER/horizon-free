//! Connection/menu-instance scoped custom recipe placement, distinct from vanilla IDs.
#[cfg(test)]
#[path = "placement_sync_tests.rs"]
mod tests;
use mod_api::wire::Wire;
use std::sync::OnceLock;

pub const RESOURCE_ID_BYTES: usize = 32767;
pub const CHUNK_BYTES: usize = super::settings_sync::MAX_BYTES / 2;
pub const MAX_CATALOG_BYTES: usize = horizon_protocol::frame::MAX_FRAME;

/// A resource location that can cross vanilla's resource-location string boundary.
#[derive(Clone, Debug, PartialEq, Eq, mod_api::Wire)]
#[wire(via = String)]
pub struct ResourceId(String);
impl ResourceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for ResourceId {
    type Error = ();
    fn try_from(value: String) -> Result<Self, ()> {
        if value.len() <= RESOURCE_ID_BYTES && super::resource_id::valid(&value) {
            Ok(Self(value))
        } else {
            Err(())
        }
    }
}
impl From<&ResourceId> for String {
    fn from(value: &ResourceId) -> Self {
        value.0.clone()
    }
}

/// Original source index inside a server-authored recipe entry, never a vanilla ordinal.
#[derive(Clone, Debug, PartialEq, Eq, mod_api::Wire)]
pub struct DisplayId {
    pub recipe: ResourceId,
    pub source: u32,
}

/// The server grants this identity only for a live, negotiated menu instance.
#[derive(Clone, Debug, PartialEq, Eq, mod_api::Wire)]
pub struct CatalogIdentity {
    pub epoch: u64,
    pub number: u64,
    pub window: i32,
    pub nonce: u64,
    pub instance: u64,
    pub generation: u64,
    pub menu: ResourceId,
    pub book: ResourceId,
}
#[derive(Clone, Debug, PartialEq, Eq, mod_api::Wire)]
pub struct Selection {
    pub catalog: CatalogIdentity,
    pub number: u64,
    pub display: DisplayId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, mod_api::Wire)]
pub enum Refusal {
    Unsupported,
    Stale,
    Unavailable,
    UnknownDisplay,
    NotPlaceable,
    Denied,
    NoRoom,
    Changed,
    TooLarge,
    Invalid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, mod_api::Wire)]
pub enum Outcome {
    Placed,
    MissingIngredients,
    Refused(Refusal),
}

#[derive(Clone, Debug, PartialEq, Eq, mod_api::Wire)]
pub enum Frame {
    Hello {
        epoch: u64,
        schema: u64,
    },
    Ready {
        epoch: u64,
        schema: u64,
    },
    Catalog {
        epoch: u64,
        number: u64,
        window: i32,
        nonce: u64,
        menu: ResourceId,
        book: ResourceId,
    },
    Start {
        identity: CatalogIdentity,
        layout: mod_api::RecipeBookLayout,
        storage: super::menu_layout::MenuLayoutMetadata,
        total: u32,
    },
    Part {
        epoch: u64,
        number: u64,
        offset: u32,
        bytes: Vec<u8>,
    },
    Place {
        selection: Selection,
        use_max: bool,
    },
    Reply {
        selection: Selection,
        outcome: Outcome,
    },
    Refused {
        epoch: u64,
        number: u64,
        window: i32,
        nonce: u64,
        reason: Refusal,
    },
    Invalidate {
        epoch: u64,
    },
}

/// Includes the opaque catalogue body and fragmentation policy, not just frame fields.
pub fn schema() -> u64 {
    static VALUE: OnceLock<u64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        let hash = mod_api::wire::layout_mix(
            Frame::layout(&mut Vec::new()),
            Vec::<mod_api::RecipeBookCategory>::layout(&mut Vec::new()),
        );
        let hash = mod_api::wire::layout_mix(hash, CHUNK_BYTES as u64);
        let hash = mod_api::wire::layout_mix(hash, MAX_CATALOG_BYTES as u64);
        mod_api::wire::layout_mix(hash, RESOURCE_ID_BYTES as u64)
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub window: i32,
    pub nonce: u64,
    pub menu: ResourceId,
    pub book: ResourceId,
}
struct PendingCatalog {
    number: u64,
    window: Window,
    incoming: Option<Incoming>,
}
struct Incoming {
    identity: CatalogIdentity,
    layout: mod_api::RecipeBookLayout,
    total: usize,
    bytes: Vec<u8>,
}
pub struct CatalogBytes {
    pub identity: CatalogIdentity,
    pub layout: mod_api::RecipeBookLayout,
    pub bytes: Vec<u8>,
}
#[derive(Default)]
pub struct Client {
    epoch: Option<u64>,
    ready: bool,
    number: u64,
    window: Option<Window>,
    pending: Option<PendingCatalog>,
    catalog: Option<CatalogIdentity>,
    completed: Option<CatalogIdentity>,
    selection: Option<Selection>,
}
impl Client {
    /// Reset connection authority before negotiating a different epoch.
    pub fn hello(&mut self, epoch: u64) -> Frame {
        if self.epoch != Some(epoch) {
            *self = Self {
                epoch: Some(epoch),
                ..Self::default()
            };
        }
        Frame::Hello {
            epoch,
            schema: schema(),
        }
    }
    pub fn ready(&mut self, epoch: u64, received: u64) -> bool {
        if self.epoch != Some(epoch) || received != schema() {
            return false;
        }
        self.ready = true;
        true
    }
    pub fn close(&mut self) {
        self.window = None;
        self.pending = None;
        self.catalog = None;
        self.completed = None;
        self.selection = None;
    }
    pub fn invalidate(&mut self, epoch: u64) -> bool {
        if self.epoch != Some(epoch) {
            return false;
        }
        self.pending = None;
        self.catalog = None;
        self.completed = None;
        self.selection = None;
        true
    }
    /// A catalogue refusal must name the outstanding request, not a superseded window.
    pub fn refused(&mut self, epoch: u64, number: u64, window: i32, nonce: u64) -> bool {
        if self.epoch != Some(epoch) {
            return false;
        }
        if number == 0 {
            if self.ready {
                return false;
            }
            self.close();
            return true;
        }
        if !self.pending.as_ref().is_some_and(|pending| {
            pending.number == number
                && pending.window.window == window
                && pending.window.nonce == nonce
        }) {
            return false;
        }
        self.invalidate(epoch)
    }
    pub fn open(&mut self, window: Window) -> Option<Frame> {
        self.close();
        if !self.ready {
            return None;
        }
        let epoch = self.epoch?;
        let number = self.number.checked_add(1)?;
        self.number = number;
        self.window = Some(window.clone());
        self.pending = Some(PendingCatalog {
            number,
            window: window.clone(),
            incoming: None,
        });
        Some(Frame::Catalog {
            epoch,
            number,
            window: window.window,
            nonce: window.nonce,
            menu: window.menu,
            book: window.book,
        })
    }
    pub fn start(
        &mut self,
        identity: CatalogIdentity,
        layout: mod_api::RecipeBookLayout,
        total: u32,
    ) -> Result<(), Refusal> {
        let pending = self.pending.as_mut().ok_or(Refusal::Stale)?;
        let total = usize::try_from(total).map_err(|_| Refusal::TooLarge)?;
        if self.epoch != Some(identity.epoch)
            || identity.number != pending.number
            || identity.window != pending.window.window
            || identity.nonce != pending.window.nonce
            || identity.menu != pending.window.menu
            || identity.book != pending.window.book
            || identity.instance == 0
            || identity.generation == 0
        {
            return Err(Refusal::Stale);
        }
        if total == 0 || total > MAX_CATALOG_BYTES {
            return Err(Refusal::TooLarge);
        }
        if pending.incoming.is_some() {
            return Err(Refusal::Invalid);
        }
        pending.incoming = Some(Incoming {
            identity,
            layout,
            total,
            bytes: Vec::new(),
        });
        Ok(())
    }
    pub fn part(
        &mut self,
        epoch: u64,
        number: u64,
        offset: u32,
        bytes: Vec<u8>,
    ) -> Result<Option<CatalogBytes>, Refusal> {
        let pending = self.pending.as_mut().ok_or(Refusal::Stale)?;
        if self.epoch != Some(epoch) || pending.number != number {
            return Err(Refusal::Stale);
        }
        let incoming = pending.incoming.as_mut().ok_or(Refusal::Invalid)?;
        let offset = usize::try_from(offset).map_err(|_| Refusal::Invalid)?;
        if offset != incoming.bytes.len()
            || bytes.len() != (incoming.total - incoming.bytes.len()).min(CHUNK_BYTES)
        {
            return Err(Refusal::Invalid);
        }
        incoming.bytes.extend(bytes);
        if incoming.bytes.len() != incoming.total {
            return Ok(None);
        }
        let incoming = pending.incoming.take().ok_or(Refusal::Invalid)?;
        self.pending = None;
        self.completed = Some(incoming.identity.clone());
        // The caller validates the complete body before accept_catalog publishes authority.
        Ok(Some(CatalogBytes {
            identity: incoming.identity,
            layout: incoming.layout,
            bytes: incoming.bytes,
        }))
    }
    pub fn accept_catalog(&mut self, identity: CatalogIdentity) -> bool {
        let Some(window) = &self.window else {
            return false;
        };
        if self.epoch != Some(identity.epoch)
            || window.window != identity.window
            || window.nonce != identity.nonce
            || window.menu != identity.menu
            || window.book != identity.book
            || self.pending.is_some()
            || self.completed.as_ref() != Some(&identity)
        {
            return false;
        }
        self.completed = None;
        self.catalog = Some(identity);
        self.selection = None;
        true
    }
    pub fn select(&mut self, display: DisplayId, use_max: bool) -> Option<Frame> {
        self.selection = None;
        let catalog = self.catalog.clone()?;
        let number = self.number.checked_add(1)?;
        self.number = number;
        let selection = Selection {
            catalog,
            number,
            display,
        };
        self.selection = Some(selection.clone());
        Some(Frame::Place { selection, use_max })
    }
    /// Only a current missing-ingredients reply authorizes ghost installation.
    /// Consume only the reply to the current selection. Stale replies cannot
    /// cancel a newer request's timeout or install its ghosts.
    pub fn reply(&mut self, selection: &Selection, outcome: Outcome) -> Option<Outcome> {
        if self.selection.as_ref() != Some(selection)
            || self.catalog.as_ref() != Some(&selection.catalog)
        {
            return None;
        }
        self.selection = None;
        Some(outcome)
    }
    pub fn click(&mut self) {
        self.selection = None;
    }
    pub fn epoch(&self) -> Option<u64> {
        self.epoch
    }
    pub fn window(&self) -> Option<&Window> {
        self.window.as_ref()
    }
    pub fn catalog(&self) -> Option<&CatalogIdentity> {
        self.catalog.as_ref()
    }
}

/// Replay protection is connection-scoped, not reset by a repeated Hello or menu close.
pub struct Authority {
    epoch: u64,
    ready: bool,
    last: u64,
    catalog: Option<CatalogIdentity>,
}
impl Authority {
    pub fn new(epoch: u64) -> Self {
        Self {
            epoch,
            ready: false,
            last: 0,
            catalog: None,
        }
    }
    pub fn hello(&mut self, epoch: u64, offered: u64) -> Result<Frame, Refusal> {
        if epoch != self.epoch {
            return Err(Refusal::Stale);
        }
        if offered != schema() {
            self.ready = false;
            self.catalog = None;
            return Err(Refusal::Unsupported);
        }
        self.ready = true;
        Ok(Frame::Ready {
            epoch,
            schema: schema(),
        })
    }
    pub fn publish(&mut self, identity: CatalogIdentity) -> Result<(), Refusal> {
        if !self.ready || identity.epoch != self.epoch || identity.number <= self.last {
            return Err(Refusal::Stale);
        }
        self.last = identity.number;
        self.catalog = Some(identity);
        Ok(())
    }
    pub fn owns(&self, catalog: &CatalogIdentity) -> bool {
        self.ready && self.catalog.as_ref() == Some(catalog)
    }
    pub fn place(&mut self, selection: &Selection) -> Result<(), Refusal> {
        if !self.owns(&selection.catalog) || selection.number <= self.last {
            return Err(Refusal::Stale);
        }
        self.last = selection.number;
        Ok(())
    }
    pub fn clear(&mut self) {
        self.catalog = None;
    }
}
