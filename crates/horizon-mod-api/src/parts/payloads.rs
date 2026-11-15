//! Concrete payload authoring; frozen lookup storage stays in the vocabulary.
use super::*;

#[cfg(test)]
#[path = "payloads/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "payloads/typed_tests.rs"]
mod typed_tests;

/// Namespaces the engine owns: `minecraft:` (`brand`, `register`, ...) and every
/// `horizon:` channel. A raw declaration cannot claim them.
const RESERVED_RAW_NAMESPACES: [&str; 2] = ["minecraft", "horizon"];

fn indices(protocol: PayloadProtocol) -> &'static [usize] {
    match protocol {
        PayloadProtocol::Configuration => &[0],
        PayloadProtocol::Play => &[1],
        PayloadProtocol::Common => &[0, 1],
    }
}

impl PartRegistrar {
    /// Validate and insert one declaration; for a typed one (`schema`, its value type's
    /// fingerprint), also its slot, whose index is returned. A refusal joins the startup refusal list, naming the mod.
    fn declare_payload(
        &mut self,
        mut declaration: PayloadDeclaration,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
        schema: Option<u64>,
    ) -> Option<(PayloadId, u32)> {
        let refused = (|| {
            if !declaration.id.0.contains(':') {
                return Err("payload ids must be explicitly namespaced".to_owned());
            }
            declaration.id.0 = self.owned_declaration_id(&declaration.id.0)?;
            if declaration.version.trim().is_empty() {
                return Err("payload version must not be blank".to_owned());
            }
            if declaration.version.contains(crate::PAYLOAD_SCHEMA_MARKER) {
                return Err(format!(
                    "payload version must not contain {:?}, which marks a typed schema",
                    crate::PAYLOAD_SCHEMA_MARKER
                ));
            }
            if declaration.direction != PayloadDirection::ToClient && server.is_none() {
                return Err("serverbound payload requires a server handler".to_owned());
            }
            if declaration.direction == PayloadDirection::ToClient && server.is_some() {
                return Err("clientbound-only payload has a server handler".to_owned());
            }
            if declaration.direction == PayloadDirection::ToServer && client.is_some() {
                return Err("serverbound-only payload has a client handler".to_owned());
            }
            if let Some(payloads) = &self.parts.payloads {
                if let Some(raw) = payloads.raw.get(&declaration.id.0) {
                    let owners: Vec<_> = raw.iter().map(|owner| owner.owner).collect();
                    return Err(format!(
                        "it collides with the raw channel declared by {}",
                        owners.join(", ")
                    ));
                }
                for &index in indices(declaration.protocol) {
                    if let Some(owner) = payloads.phases[index].get(&declaration.id.0) {
                        return Err(format!(
                            "duplicate payload registration for {} in protocol {index}, already registered by {}",
                            declaration.id.0, owner.owner
                        ));
                    }
                }
                // One id has one value type: a typed id lives in one declaration, whose
                // protocol a token names, so no other protocol may declare it.
                let typed =
                    schema.is_some() || payloads.typed.iter().any(|slot| slot.id == declaration.id);
                if typed {
                    for (index, phase) in payloads.phases.iter().enumerate() {
                        if let Some(owner) = phase.get(&declaration.id.0) {
                            return Err(format!(
                                "a typed payload has one declaration, and {} is already registered in protocol {index} by {}",
                                declaration.id.0, owner.owner
                            ));
                        }
                    }
                }
            }
            Ok(())
        })();
        if let Err(reason) = refused {
            self.refused.push(format!(
                "{} registered payload {}: {reason}",
                self.mod_id, declaration.id.0
            ));
            return None;
        }
        let (owner, owner_index) = (self.mod_id, self.mod_index);
        let payloads = self.payloads_table();
        let slot = schema.map(|schema| {
            declaration.version = crate::typed_payload_version(&declaration.version, schema);
            payloads.typed.push(TypedSlot {
                id: declaration.id.clone(),
                protocol: declaration.protocol,
                direction: declaration.direction,
                schema,
                owner_index,
            });
            (
                declaration.id.clone(),
                u32::try_from(payloads.typed.len() - 1)
                    .expect("fewer typed payloads than u32::MAX"),
            )
        });
        let server_panicked = Arc::default();
        let client_panicked = Arc::default();
        for &index in indices(declaration.protocol) {
            let mut normalized = declaration.clone();
            normalized.protocol = if index == 0 {
                PayloadProtocol::Configuration
            } else {
                PayloadProtocol::Play
            };
            payloads.phases[index].insert(
                declaration.id.0.clone(),
                Registered {
                    declaration: normalized,
                    owner,
                    owner_index,
                    server: server.clone(),
                    client: client.clone(),
                    client_thread: declaration.thread,
                    server_panicked: Arc::clone(&server_panicked),
                    client_panicked: Arc::clone(&client_panicked),
                },
            );
        }
        slot
    }

    /// This load's payload table, made on first use: the one place a table is created, so
    /// every table has a nonzero load of its own and a token of another load is stale.
    fn payloads_table(&mut self) -> &mut Payloads {
        static NEXT_LOAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Arc::make_mut(self.parts.payloads.get_or_insert_with(|| {
            Arc::new(Payloads {
                load: NEXT_LOAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                ..Payloads::default()
            })
        }))
    }
}

impl PayloadsRegistrar for PartRegistrar {
    fn add_payload(
        &mut self,
        declaration: PayloadDeclaration,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    ) {
        self.declare_payload(declaration, server, client, None);
    }

    fn add_typed_payload(
        &mut self,
        declaration: PayloadDeclaration,
        schema: u64,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    ) -> PayloadToken {
        let (protocol, direction) = (declaration.protocol, declaration.direction);
        let requested = declaration.id.clone();
        let (id, slot, load) = match self.declare_payload(declaration, server, client, Some(schema))
        {
            Some((id, slot)) => (
                id,
                slot,
                self.parts
                    .payloads
                    .as_ref()
                    .map_or(0, |payloads| payloads.load),
            ),
            None => (requested, u32::MAX, 0),
        };
        PayloadToken::new(id, slot, load, protocol, direction, schema)
    }

    fn add_client_payload_handler(
        &mut self,
        id: &PayloadId,
        thread: HandlerThread,
        handler: Arc<dyn IPayloadHandler>,
    ) {
        let Some(payloads) = self.parts.payloads.as_mut() else {
            self.refused.push(format!(
                "{} supplied a client handler for unknown payload {}",
                self.mod_id, id.0
            ));
            return;
        };
        let mut found = false;
        for phase in &payloads.phases {
            if let Some(entry) = phase.get(&id.0) {
                if entry.owner != self.mod_id {
                    self.refused.push(format!(
                        "{} cannot attach a client handler to {} owned by {}",
                        self.mod_id, id.0, entry.owner
                    ));
                    return;
                }
                if entry.declaration.direction != PayloadDirection::ToServer {
                    found = true;
                    if entry.client.is_some() {
                        self.refused.push(format!(
                            "{} registered a second client payload handler for {}",
                            self.mod_id, id.0
                        ));
                        return;
                    }
                }
            }
        }
        if !found {
            self.refused.push(format!(
                "{} supplied a client handler for unknown or serverbound-only payload {}",
                self.mod_id, id.0
            ));
            return;
        }
        let panicked = Arc::default();
        for phase in &mut Arc::make_mut(payloads).phases {
            if let Some(entry) = phase.get_mut(&id.0)
                && entry.declaration.direction != PayloadDirection::ToServer
            {
                entry.client = Some(handler.clone());
                entry.client_thread = thread;
                entry.client_panicked = Arc::clone(&panicked);
            }
        }
    }

    fn add_raw_payload(&mut self, id: PayloadId, server: Option<Arc<dyn IPayloadHandler>>) {
        let refused = (|| {
            let parsed = Identifier::parse_namespaced(&id.0)
                .ok_or_else(|| "a raw channel needs an exact namespaced id".to_owned())?;
            if RESERVED_RAW_NAMESPACES.contains(&parsed.namespace()) {
                return Err(format!(
                    "the {:?} namespace belongs to the engine",
                    parsed.namespace()
                ));
            }
            let Some(payloads) = &self.parts.payloads else {
                return Ok(());
            };
            for phase in &payloads.phases {
                if let Some(typed) = phase.get(&id.0) {
                    return Err(format!(
                        "it collides with the typed payload registered by {}",
                        typed.owner
                    ));
                }
            }
            if payloads
                .raw
                .get(&id.0)
                .is_some_and(|owners| owners.iter().any(|owner| owner.owner == self.mod_id))
            {
                return Err("it is already declared by this mod".to_owned());
            }
            Ok(())
        })();
        if let Err(reason) = refused {
            self.refused.push(format!(
                "{} declared raw channel {}: {reason}",
                self.mod_id, id.0
            ));
            return;
        }
        let (owner, owner_index) = (self.mod_id, self.mod_index);
        self.payloads_table()
            .raw
            .entry(id.0)
            .or_default()
            .push(RawRegistered {
                owner,
                owner_index,
                server,
                panicked: Arc::default(),
            });
    }
}
