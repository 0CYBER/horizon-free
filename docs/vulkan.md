# Compare the Vulkan renderer on your Mac

The `vulkan` profile asks Minecraft 26.3 to use its existing Vulkan backend, with
its original OpenGL fallback. It uses balanced mods/distances and omits Iris.
The game renderer handles the backend; this project has not written a separate
Metal renderer. The benchmark records the actual backend and driver.

The checksum-verified 26.3 manifest includes LWJGL Vulkan 3.4.3 Mac ARM64 natives.
Inspecting that official native JAR confirmed `libMoltenVK.dylib`; its paired
LWJGL loader first tries MoltenVK on macOS. [MoltenVK](https://github.com/KhronosGroup/MoltenVK)
implements Vulkan functionality over Apple's Metal framework. This verifies a
bundled route to Metal, rather than proving that a particular Mac backend starts
or performs better. No separately installed Vulkan SDK is required for the bundled
library; driver/backend compatibility still needs a real Mac test.

Import `m2-balanced` and `vulkan` as separate Prism instances. Keep Java 25 ARM64,
heap, header/collector flags, world copies, resolution, view/simulation distance
and camera route equal. Warm terrain and shader/pipeline caches before comparing.
Run at least three alternating 120-second F8 recordings. Inspect nearby blocks,
distant terrain, transparency, water, lighting and reloads as well as timings.
Use the viewer's actual backend field: OpenGL fallback is an OpenGL result.

After the isolated test works, `foreground-first` and `foreground-vulkan` compare
the two backends with the same combined terrain controls. Their 120 FPS adaptive
goal controls background duty cycle; rendering remains uncapped in pack options.
Do not add Iris or shaders to one side of an isolated backend comparison.

Linux software rendering checks reached the actual Vulkan backend, rendered the
demo world and recorded matching CSV/JSON frame/tick samples. An unavailable
Vulkan-driver test reached the menu using OpenGL fallback. These tests do not
validate MoltenVK, real GPU speeds, all visual details or macOS 27. The user's
M2 Pro remains the priority device for those checks.
