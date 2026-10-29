# Windows allocator experiments

The normal client uses Rust's system allocator and does not request Windows
Segment Heap. Two independent, off-by-default Cargo features allow experiments:

| Feature | Effect on Windows |
| --- | --- |
| `windows-mimalloc` | Uses `mimalloc::MiMalloc` as the `horizon-client` executable's Rust global allocator. |
| `windows-segment-heap` | Embeds a manifest requesting Segment Heap for Win32 heap APIs in this package's executables. |

Neither feature changes non-Windows builds. These are build options, not mods or
runtime settings; changing either requires rebuilding and restarting the client.
Enabling both does not make every allocation use mimalloc: native libraries can
still use Windows heap APIs. No frame-rate improvement is guaranteed.

```sh
# Baseline, then each option separately, then both. Keep the same profile.
cargo build --profile fast -p horizon-client --bin horizon-client
cargo build --profile fast -p horizon-client --bin horizon-client --features windows-segment-heap
cargo build --profile fast -p horizon-client --bin horizon-client --features windows-mimalloc
cargo build --profile fast -p horizon-client --bin horizon-client --features windows-segment-heap,windows-mimalloc
```

Copy each executable to a separate experiment directory before building the next
variant, or use a separate `CARGO_TARGET_DIR`. Keep prepared resources and worlds
outside version control. Use disposable copies of a world and isolated data
directories for tests, following [AGENTS.md](../AGENTS.md).

Segment Heap is supported starting with Windows 10 version 2004 (build 19041),
according to Microsoft's [application manifest documentation](https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests#heaptype).
The manifest leaves the optional architecture unspecified and keeps the usual `asInvoker`
execution level. The existing icon and version resources are compiled together
with it through the existing Windows-host resource build step. Cross-compiling
that manifest from a non-Windows host is not supported by this build step.
Builds on earlier Windows versions have not been tested.

## Validate the executable

On Windows, extract the manifest from a built executable using the Windows SDK's
`mt.exe` (PowerShell syntax):

```powershell
mt.exe '-inputresource:target/fast/horizon-client.exe;#1' '-out:client.manifest'
mt.exe -manifest client.manifest -validate_manifest
```

Check that the feature build contains `SegmentHeap` in the
`http://schemas.microsoft.com/SMI/2020/WindowsSettings` namespace. The baseline
must not contain this element. Confirm Explorer still shows Horizon's icon for
both builds. Inspecting the final executable matters: a valid source XML file
alone does not prove that the linker embedded it.

## Measure separately

Use the same commit, compiler, profile, settings, driver configuration, world
copy, warm-up period and flight route for all four variants. Alternate their
order across repeated sessions. Capture individual frame times, their p99 and
maximum, memory use and average FPS. Record game FPS limits, the display refresh
rate and driver-level VSync: a driver override can cap FPS even when game VSync
is disabled.

These features let measurements distinguish the Rust allocator from the Win32
heap choice. Results from builds that also change meshing or LOD retirement
cannot establish either allocator's individual contribution.
