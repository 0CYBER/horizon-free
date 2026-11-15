# Permission nodes and handlers: proposed design

**Status: the approved design for arismoko/horizon#1034 (merged in arismoko/horizon#2876); the build is arismoko/horizon#1034 and is not done.** The two gather hooks and the query API described here do not exist yet; the build follows this document. It does not change sound hooks or existing permissions.

## Boundary and existing contracts

A permission node is a named, typed question a server-side mod explicitly asks about a player UUID. A default supplies the answer when no external policy is selected. Exactly one selected handler answers every registered node for that server; handlers are not a chain of voters. This is a platform service, not an engine dependency. An unmodified game and its vanilla checks must behave as before. The [layer split](../../layers.md) and [mod API design](direction.md#registering-behaviour) put collection and ownership in the game and prohibit an engine dependency on the mod platform.

Today commands use operator levels, not nodes: `Command::level` is 0–4, with 0 available to everyone (`crates/horizon-mod-vocab/src/command/types.rs`); the command tree and dispatch enforce their existing levels. Existing game-master block checks also use levels (`crates/horizon-client/src/game/administration/command_blocks.rs`). No vanilla command, command tree, command-block restriction or other existing check is implicitly converted to a node. A mod that wants a node check must call it at its own server-side authorization point; a node grant does not grant an operator level, and a node denial does not remove one. A mod command that declares a node follows the answer order in "Command paths" below instead; commands without a node keep level enforcement unchanged, and vanilla's own commands are untouched.

The existing `permissions_changed` hook is different: the catalog declaration is `crates/horizon-mod-vocab/src/hooks/permissions_changed.rs`, with dispatch target `crates/horizon-client/src/game/native_hooks/permissions_changed.rs`. On dedicated servers it can cancel a pending `PlayerList.op`/`deop` operator-list change, including for an offline player; `operators/clear` is not such a call. It neither reports nor vetoes individual node decisions, and node decisions do not fire it. A canceled level change leaves the old level in force; a committed one affects the next check using that player's current level. A handler must not assume this hook reports all policy or operator-list changes. No node decision cache is specified, so there is no cache to invalidate through this hook.

## Nodes and queries

Node identity is its case-sensitive dot-separated name plus value type, not name alone. Built-in value types include Boolean, integer, string and text component. There is no implicit hierarchy, wildcard matching or namespace ownership restriction. Duplicate typed nodes are registration errors, even within one mod; the same name with different types denotes different nodes.

Each node declares a mod-written default resolver receiving an optional online player, the player's UUID and dynamic contexts. Context keys are typed and have serialization contracts. Undeclared context keys are ignored; not every declared key must be supplied. Defaults are evaluated at query time, not gathering time. Online queries and offline UUID queries return the node's typed value or an explicit error. An unregistered query is an error, never an implicit registration or grant. Clients cannot authorize server actions.

## Collection and selection

On every logical server start, after levels load and before server-starting:

1. Gather handler factories, including the built-in default factory. Duplicate handler ids are errors.
2. Choose a factory by server configuration. No configured id selects the default. An unknown configured id selects the default with a diagnostic, rather than failing startup.
3. Gather typed nodes. Duplicate typed nodes or invalid mod input fail registration without panicking.
4. Create the selected handler from the complete validated node set and publish the service atomically.

Queries are unavailable before initialization. No late registration is allowed. Closing a server discards its service; reopening gathers afresh. Joins and resource reloads do not recollect. Dispatch uses the shared native/wasm per-mod dependency order, not a bundle packaging model. Native and wasm share this contract and the same failure boundaries.

## Handler contract and failures

Exactly one selected handler answers each query with the node's typed value. The built-in handler invokes the node's mod-written resolver with the optional online player, UUID and contexts; there is no universal Allow/Deny/UseDefault result. A custom handler can supply policy independent of operator levels. Node decisions do not fire `permissions_changed` or change vanilla operator levels.

An unregistered node, invalid value, wrong-server online handle, handler error, trap or recursive query yields an explicit query error. Failures never switch handlers or invoke a potentially permissive default. Recursion fails both inner and enclosing queries. The protected operation must handle errors safely; integer, string and component errors must not be converted into Boolean denial values. Diagnostics identify the node and handler. There is no answer cache, implicit world write or transaction during a query.

## Command paths (maintainer ruling)

A mod command may declare a **per-path requirement** that consults the typed service with a typed node (never a bare string). It is the command's own requirement check, so vanilla's existing behaviour follows: a path whose check fails is left out of the command tree sent to that player and answers as unknown when typed. There is no separate hiding mechanism.

Answer order for a command that declares a node: a configured handler's answer decides; with no handler configured the usage's operator level decides, as in vanilla; a handler error denies and is logged, and never falls back to the level. Vanilla's own commands, and mod commands without a node, are unchanged.

One server host method resends a player's command tree (vanilla already resends it on join, respawn, dimension change and an operator change); a handler calls it when its answers change. The server rechecks on every run, so a stale tree affects suggestions only. This is built on top of the typed service (arismoko/horizon#3609).

## Example mod (contract sketch)

This is pseudocode, not an available or compilable API:

```text
NODE = Node<Boolean>("garden.harvest_rare",
    resolver = (online, uuid, contexts) => online.map(p => p.operator_level >= 2).unwrap_or(false))
permission_gather_handler(factories):
    factories.add("garden.policy", nodes => GardenPolicy(nodes))
permission_gather_nodes(nodes):
    nodes.add(NODE)
GardenPolicy.query(node, online, uuid, contexts):
    if node == NODE and uuid == A: return true
    if node == NODE and uuid == B: return false
    return node.resolver(online, uuid, contexts)
harvest(player):
    match permissions.query(NODE, player):
        Ok(true): harvest_crop()
        Ok(false) or Err(error): refuse_harvest()
```

The administrator selects `garden.policy`. A is allowed at level 0; B is denied at level 4. Other UUIDs use the resolver, including offline UUIDs with no online handle. None of this changes `/op` or vanilla commands.

## Acceptance matrix for an implementation

These are future tests, not claims of implemented behavior. Run equivalent native and wasm cases on dedicated and integrated servers.

| Setup / action | Expected observable result |
| --- | --- |
| Query Boolean, integer, string and component nodes | Each returns its declared type. |
| Register the same name with distinct types | Distinct nodes coexist. |
| Register the same typed node twice | Registration error, no partial service. |
| Query unregistered node | Explicit error before handler invocation. |
| Query online and offline UUIDs | Resolver receives UUID, optional online handle and contexts. |
| Supply undeclared contexts; omit declared contexts | Undeclared keys ignored; omissions allowed. |
| Serialize a declared dynamic context | Typed key's serialization contract preserved. |
| Configure missing handler id | Default factory chosen with diagnostic. |
| Duplicate handler ids | Registration error, no partial service. |
| Observe startup callbacks | Factories, selection, nodes, construction after levels load and before server-starting. |
| Query before initialization or register late | Refused without panic. |
| Handler fails or recurses | Explicit typed query error, no fallback grant. |
| Close and reopen server | Fresh factories, nodes and handler; no shared server state. |
| Join or reload resources | No re-gathering. |
| Cancel or commit operator change through permissions_changed | Resolver sees the unchanged or updated current level; node query does not fire that hook. |
| Run vanilla commands with no node consumers | Existing operator behavior unchanged. |
