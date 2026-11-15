# Design: main target configuration

Provisional answer to [arismoko/horizon#869](https://github.com/arismoko/horizon/issues/869):
configure one renderer-owned main colour attachment and mandatory depth attachment,
before allocation, through `configure_main_render_target` with `ClientHost`.
The initial supported change proposed here is adding stencil through a typed
`depth_stencil` choice. This requires renderer work; it is not available today.
The colour format is never offered. This is a review proposal, not an adopted
claim of full NeoForge parity.

## Evidence and limits

Vanilla 26.3 was inspected only through `tools/rea_vanilla.py` from
`/workspace/horizon`: `class MainTarget` (fully qualified
`com.mojang.blaze3d.pipeline.MainTarget`), and methods `allocateAttachments`,
`allocateColorAttachment`, `allocateDepthAttachment`, plus callers of
`createFrameBuffer`. Allocation asserts the render thread, tries dimensions with
fallback, and requires both colour and depth before creating their views.
Colour allocation uses the target's colour format, one mip and one layer;
allocation failure returns null for fallback. These are method owners, not
invented Java file:line references. No Mojang files are included here.

NeoForge 26.3's issue-named counterpart is `ConfigureMainRenderTargetEvent`.
Its exact fields, defaults, mutability, bus and recreation timing remain
**unverified**: the public [26.3 javadoc endpoint](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/client/event/ConfigureMainRenderTargetEvent.html)
was inaccessible on 2026-11-09. No NeoForge source or patches were consulted.
Do not adopt this narrower field set as its full contract: public documentation
of each field and timing is a prerequisite to approving implementation.

Verified Horizon owners at this revision (all paths relative to the repository):

- `crates/horizon-client-render/src/gfx.rs:2911`: renderer construction;
  `:3082`: frame configuration selection before pipeline construction.
- `gfx.rs:5515`: offscreen target helper, single sample, one mip/layer,
  renderer-selected format and usage.
- `gfx.rs:5582`: depth helper, `Depth32Float`, single sample, render,
  sample and copy-source usage; depth consumers must survive any proposed change.
- `gfx.rs:5640`: resize validates against device maximum, suspends invalid sizes,
  then configures surface or recreates offscreen target and recreates depth.
- `gfx.rs:5547`: surface restoration; `:2651`: device-loss callback;
  `:2354`: frame configuration policy. Surface configuration alone is not proof
  that all dependent targets and pipelines have been rebuilt.

## Proposed fields and refusals

| Field | Supported values | Ownership |
|---|---|---|
| `depth_stencil` | `Depth`, `DepthStencil` | Writable; default `Depth`; stencil default clear 0 |
| extent | Current drawable width and height | Read-only renderer decision |
| colour attachments | Exactly one main colour | Renderer-owned |
| samples, mips, layers | 1, 1, 1 | Renderer-owned |
| depth presence | Always true | Renderer-owned |
| colour format, primaries, transfer, alpha | No writable field | Renderer-owned |
| depth format and usage bits | No raw format or flags | Renderer chooses a supported compatible format |

Refuse extra colour attachments, MSAA, depth removal, custom dimensions,
custom usages, raw GPU objects, clear-colour overrides and colour/alpha policy
requests as `UnsupportedMainTargetCapability` at registration, identifying the
mod and requested capability. A required unsupported capability prevents that
registration; never silently ignore it. This refusal owns only this feature's
capability identifiers. Unknown identifiers elsewhere retain existing behaviour.
Stencil is advertised only when the renderer can support it throughout all
passes, depth sampling and copies; otherwise its registration is refused too.
No arbitrary fixed dimension ceiling: preserve renderer/device capacity and
vanilla's requested-size/fallback behaviour rather than imposing a mod-only cap.

The API owns `MainTargetConfigWire`, `MainTargetDepthStencilWire` and
`MainTargetCapabilityWire` with explicit discriminants and checked decoding;
no wgpu types or Rust memory layouts cross the wasm boundary. Native and wasm
use the same configuration, capability negotiation, dependency-ordered listener
sequence and validation. Unknown discriminants, oversized payloads and malformed
values are refused before callbacks or allocation. There are no floating point
mod fields; NaN/infinity cannot enter through a numeric escape hatch.

## One recreation boundary

Client-lib decides a candidate generation with extent, device capabilities,
colour policy and reload generation. The game dispatches with `ClientHost`,
then applies the validated result in the same transaction, rechecking those
preconditions without recomputing the decision. Each listener sees the preceding
validated result; the last valid write wins. Invalid output refuses the whole
candidate. Catch a callback unwind or wasm trap at the runtime boundary, discard
its transaction and refuse the candidate with mod attribution. A native process
abort cannot be contained. Do not resume allocation using partially edited state.

Validate the whole attachment/pipeline combination before GPU allocation, then
build a replacement generation and atomically publish it with matching views,
bind groups, generated colour boundaries and pipelines. Allocation failure drops
the candidate and retains the old usable generation; first creation fails with
a diagnostic. Device loss may make retention impossible: suspend rendering.

Use this boundary for initial window and offscreen creation, valid resize,
restoration that replaces attachments, SDR/HDR changes, float-frame selection,
shader-pack/resource reload affecting target consumers, and device recovery.
Pure surface reacquisition or present-mode changes reuse the current decision
if no attachment dependency changes. Zero-sized/minimized or over-device-limit
windows suspend without invoking listeners or allocating; restoration makes one
fresh decision for the valid size. Reload rebuilds resources and generated
entry/exit conversions together. Mods receive no persistent GPU references.
The [colour rule](mod-render-colour.md) remains linear light, white 1.0,
values above 1 allowed, with renderer-owned frame format and alpha.

No-listener dispatch is an O(1) subscriber-presence check and uses existing
renderer defaults without allocating a Wire message or scanning mods. It adds
no per-frame callback, allocation or capability scan. Cache validated capabilities
per device generation; recreation alone runs subscribed callbacks.

## Acceptance matrix (implementation gates)

| Case | Concrete expected result |
|---|---|
| No listener, 1280 × 720 | 1 colour, depth present, samples/mips/layers 1/1/1; existing formats unchanged |
| Supported stencil request | `DepthStencil`, clear stencil 0, depth sampling/copy consumers valid |
| Resize to 1920 × 1080 | One new decision; stencil retained by listener; all views use 1920 × 1080 |
| Resize to 0 × 0 | Zero callbacks, zero allocations; rendering suspended |
| Maximum supported extent | Accepted if renderer can allocate; no extra mod ceiling |
| Extra colour or samples 4 request | `UnsupportedMainTargetCapability` at registration, zero callbacks |
| Unsupported stencil device | Same named registration refusal, no partial allocation |
| Invalid Wire tag / callback panic / wasm trap | Candidate refused; no publication or partial Host writes |
| SDR → HDR → float change | New generation each effective policy change; shader white stays 1.0, bright input 2.0 stays linear 2.0 at shader boundary |
| Resource reload / device recovery | Fresh capability validation as needed, no stale views or pipelines |
| Allocation failure during resize | Candidate dropped; old generation retained if usable |
| Native versus wasm | Identical accepted settings, order, refusals and resulting descriptor values |

## Proposed stages

File: none for implementation in this change. Stage 1: obtain public NeoForge
26.3 field/timing documentation and approve the supported/refused table.
Stage 2: implement the API-owned Wire schema and capability registration,
then the shared decide/fire/apply boundary across every route above.
Stage 3: prove the matrix with descriptor tests, failure injection and native/wasm
fixtures; demonstrate stencil changing an observable render and surviving resize
in game. This docs-only change provides no such implementation proof.

## Open questions

- Does NeoForge expose more mandatory fields or different timing? Recommendation:
  require public 26.3 documentation before adopting the contract; reconcile all
  fields explicitly rather than claiming parity from the issue summary.
- Can stencil coexist with every current depth-copy consumer? Recommendation:
  gate capability advertisement on end-to-end validation; do not advertise it
  merely because the device supports a stencil format.
- Should resource reload always invoke the hook? Recommendation: invoke once
  when rebuilding attachment consumers; reuse the generation for unrelated reloads.
- How should conflicting listeners be diagnosed? Recommendation: dependency order,
  last validated write wins, with mod-attributed diagnostics for rejected candidates.
