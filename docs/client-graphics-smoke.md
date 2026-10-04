# Client startup and world-rendering probes

Compilation does not show whether a client creates its graphics device, reaches
the menu, enters a world or reads back a rendered frame. The private graphics
probe runs the actual pinned game and performance mods to check those steps.
It is a separate test mod, excluded from production JARs and playable packs.

The required macOS CI cases use the recommended OpenGL `core-performance` and
`m2-balanced` profiles. Each profile starts at the title screen and then, in a
separate instance, creates or opens an ordinary limited-demo world. World mode
dismisses the demo-help popup, waits for focused, unpaused gameplay, reads back
the main render target, and stops the client so its integrated server saves.
The readiness counter resets while a loading overlay is present: a title screen
object can already exist underneath asynchronous loading. Private screenshots
must have a nonuniform sample grid and a verified file hash;
world mode also checks for saved world data after a clean process exit.

The startup wrapper observes backend library/device creation and preserves the
original calls and thrown exceptions. Selected diagnostics contain initialization
stages, bounded exception type names, Vulkan error-code identifiers and device
labels. Failed probes also select up to eight graphics stack method names, without
module prefixes, filenames, line numbers or messages. Those observed locations
do not by themselves identify a root cause. Expected demo authentication errors
are reported separately. Full log
messages, raw screenshots, assets and world files stay under gitignored `.local/`.
Nonuniform pixels are not proof of correct visuals, full LOD completion or
exhaustive gameplay equivalence.

The [selected live observations](../benchmarks/client-graphics-2026-10-04/live-observations.json)
record passing title and demo-world readbacks for both profiles on the hosted
macOS ARM64 runner. Both world cases are focused, produce saved data and exit
cleanly. All four CI jobs pass for that source head. Its independently built
macOS production JAR matches published 0.20.0 byte-for-byte and contains neither
probe class. The observed macOS OpenGL device is **Apple Software Renderer**;
these are platform runtime checks, not physical Apple GPU performance results.

## Run locally

Build the original mod first, then use the platform's display:

```sh
python3 tools/build_mod.py
python3 tools/test_client_graphics.py --native-display
python3 tools/test_client_graphics.py --native-display --world
```

These default to core and balanced, with a 3 GiB heap. That budget can trigger
DH's low-memory warning; the 16 GB Mac starter suggests 6 GiB. A short startup
check is not a recommendation for sustained memory or distant-generation load.
On headless Linux, omit `--native-display` to use Xvfb, or run through `vscreen`
with its display available. World mode creates private, disposable demo worlds
in its isolated runtime folders. No Microsoft credentials are needed for the
game's limited demo. It does not accept the dedicated-server EULA.

The selected output is `dist/client-graphics-smoke.json`. Each case records the
observed backend rather than inferring it from the preference. Normal CI retains
separate title/world JSONs. Neither the fixture nor screenshots enter releases.

## Vulkan investigation

The manual **macOS Vulkan research** workflow separately probes vanilla, core and
the distant-terrain Vulkan profile. It runs every case, retains selected successful
and failed outcomes, and exits unsuccessfully when a case fails. Its exploratory
results do not count as passing the required OpenGL checks.
Its optional `world` input selects disposable demo-world probes for every case;
the default still probes titles.

Local Linux software OpenGL/Vulkan title readbacks passed. Early macOS OpenGL
title checks for core and balanced also passed with **Apple Software Renderer**.
The macOS runner's **Apple Paravirtual device**, using MoltenVK 1.4.2, created
Vulkan devices but all three Vulkan cases failed with `VK_ERROR_FEATURE_NOT_PRESENT`.
The failure also occurs with vanilla; these observations do not isolate an
optimization-mod fault or prove behavior on a physical M2 GPU.

The separate [readback-stage observations](../benchmarks/mac-vulkan-2026-10-04/live-observations.json)
from manual run 37193558884 show that all three cases reached the client test
initializer and requested readback before failing. They produced no completed
readback report. This narrows the observed sequence; it does not establish whether
the screenshot operation itself or subsequent rendering failed. Graphics stack
locations from the next run, 37193954843, place each observed failure in the
screenshot callback after Vulkan submission. Later probes also record callback
entry, a whitelist of numeric readback dimensions/sample counts, and fixed failure
categories from our own assertions. Separately logged Vulkan codes are not
treated as the cause of a probe assertion.

Run 37194354615 identifies the actual probe assertion: each callback received a
1280×646 image, but its 4,160 sampled pixels had only one distinct RGB colour.
The strict nonuniform-readback assertion rejected all three cases. Overlay state
was not recorded in those failures; later probes wait for its removal before
counting readiness. This tests an initialization hypothesis while preserving the
same nonuniform-image requirement. It is not evidence that the game's displayed
Vulkan window was uniform or that a physical M2 GPU has the same problem.

After the overlay guard, run 37194690645 has mixed results: core and distant-terrain
title probes pass on the Apple Paravirtual Vulkan device, with no overlay and
nonuniform readback, then exit cleanly. Vanilla still fails the uniform-readback
assertion. The overall research run therefore remains failed. These outcomes
justify retaining the readiness guard; they do not establish the cause of the
remaining failure, full visual correctness, world rendering or physical M2 speed.

The subsequent world-mode run 37195149166 passes all three cases on the hosted
Apple Paravirtual Vulkan device. Each reaches focused gameplay without an overlay,
returns a nonuniform framebuffer, produces saved demo-world data and exits with
code zero. This verifies short Vulkan world startup on that runner, while the
failed title observations remain retained. A local Linux core Vulkan world probe
also passes, and its private screenshot was visually inspected. These checks do
not prove exhaustive graphics/gameplay equivalence, completed distant terrain,
physical M2 GPU compatibility or an FPS improvement.

For an isolated private preference override:

```sh
python3 tools/test_client_graphics.py --native-display --profile baseline --backend vulkan
python3 tools/test_client_graphics.py --native-display --profile core-performance --backend vulkan
```

Overrides use separate runtime folders. The shipped profiles retain their
configured preferences. These probes make no FPS, CPU/RAM reduction, hardware-GPU
compatibility or M2 Pro performance claim. Actual-world performance comparisons
still use the [Mac first-test procedure](mac-first-test.md).
