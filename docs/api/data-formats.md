# Data formats

The files Horizon writes that other tools may read: benchmark reports, settings, mods, release checksums
and worlds. Each entry names its version field, its documentation and its level ([stability.md](stability.md)).

## Overview

| Format | Where | Version | Level | Documented in |
| --- | --- | --- | --- | --- |
| Benchmark report | the benchmark's output, the local log | `"format": "horizon-benchmark-report"`, `"version": 1` | stable | [benchmark-report.md](../benchmark-report.md) |
| Content mod package | `.hmod`: a zip with `horizon-mod.toml`, `assets/`, `data/` and optional `icon.png` | the manifest's `api` | preview | [../modding/src/reference/manifest.md](../modding/src/reference/manifest.md), [mods.md](../modding/src/how-to/content-mods.md#the-manifest), [../modding/src/reference/registry-index.md](../modding/src/reference/registry-index.md) |
| Native mod package | `.hmod` with native code: the same flat package plus root `Cargo.toml` and `src/`, with `[native]` in `horizon-mod.toml` | the manifest's `api` for content; exact `horizon` and `mixins` source pins for code | experimental | [../modding/src/reference/manifest.md](../modding/src/reference/manifest.md), [../modding/src/how-to/native-mods.md](../modding/src/how-to/native-mods.md) |
| Mods turned on | `horizon-mods.txt` | none | preview | [mods.md](../modding/src/start/installing.md#playing-with-mods) |
| Horizon settings | `horizon-graphics.txt`, `horizon-extras.txt` (`key:value`, as `options.txt`) | none: unknown keys fall back to defaults | preview | [graphics.md](../graphics.md), [extras.md](../extras.md#options-storage) |
| Server settings | `server.toml` | its keys | stable | [server-reference.md](server-reference.md) |
| Control endpoints | the game's socket file (TCP form), `.horizon/api.json` | none | preview | [agent-control.md](agent-control.md), [server-reference.md](server-reference.md) |
| Release checksums | `SHA256SUMS`, `SHA256SUMS.sig` on releases since signing began (#1011) | the signed text names the tag | stable | [security.md](../security.md) |
| Worlds | the world folder | vanilla 26.3's (`DataVersion`) | vanilla's | [world-data.md](../world-data.md) |

Worlds are vanilla's format, so any tool that reads a 26.3 world reads a Horizon world; Horizon adds itself to
`level.dat`'s `ServerBrands` as vanilla's server does.

## Rules for every format

- A reader ignores fields it doesn't know, and refuses a version newer than it knows with a clear message.
- An optional field may be added in the same version; renaming, removing or changing the meaning of a field makes the
  next version, and Horizon's own readers keep reading the older ones for the deprecation window.
- Each versioned format has a sample file of every version its reader accepts, read by a test.

## Not there yet

- Versions for the settings files and the control endpoint files (a `version` line or field).
- Sample files and reader tests for every versioned format.
- A generated schema for the benchmark report from their Rust types, checked by a test like
  the references.

## Changelog

- Benchmark report version 1. The telemetry batch and consent formats are gone: nothing is sent.
- Content mods can declare mob effects in `data/<id>/horizon/mob_effect/<name>.json` (`category`, `color`).
- Content mods can declare how a fluid looks on the client in `data/<id>/horizon/fluid/<name>.json` (`sprite`, an atlas sprite id, and `tint`, `0xAARRGGBB`, both optional), as NeoForge's `IClientFluidTypeExtensions`; the fluid needs no block.
