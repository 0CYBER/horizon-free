# What Horizon is

Horizon is Minecraft: Java Edition 26.3 rewritten in Rust. You install it with a small launcher, and it runs on the game files from your own copy of Minecraft. It is free, open source and unofficial.

## In one minute

- **A new game program, not a mod.** Horizon is written from scratch in Rust. It reads the same worlds, plays by the same rules and generates the same terrain as the Java game.
- **Checked against vanilla.** {{HARNESSES}} automated tests ("harnesses") feed the same inputs to Horizon and to the real game, then compare the results. Today every subsystem matches in every harness that compares it. That is {{PERCENT}} of what the harnesses compare, which is not proof that nothing else differs. The [status table](/#parity) lists what is covered.
- **Light and fast.** It starts quickly, uses little memory and draws efficiently. See [Performance](/docs/performance) for what is measured and what is not.
- **A few things vanilla doesn't have.** Its own settings, added to vanilla's menus the way mods add theirs, graphics presets, distant terrain, controller support, optional extras such as zoom, a minimap and dynamic lights, an invite code to play your world with friends, and a server you can run without Java.

## What Horizon is not

- **Not Minecraft itself.** Horizon contains no Minecraft code, assets or data, and it never downloads them. You need your own legally obtained copy of Minecraft: Java Edition 26.3. Horizon builds its game files from that copy, on your computer.
- **Not a place where Java mods run.** Forge, NeoForge, Fabric and Quilt mod jars do **not** work in Horizon, and neither do Mixins. Horizon has its own mod system, and the launcher can port some Java mods' content. See [Mods and data packs](/docs/mods) and [Modpacks](/docs/modpacks).
- **Not for other versions.** Horizon plays Java Edition 26.3. Worlds from older versions (1.8 to 26.2) are upgraded the way vanilla upgrades them.
- **Not an official Minecraft product.** It is not approved by or associated with Mojang or Microsoft.
- **Not able to sign in to a Microsoft account yet.** Until it can, Horizon cannot join servers that check accounts. See [Multiplayer](/docs/multiplayer).

## Supported systems

- **Linux, x86-64.** A graphics card with Vulkan drivers. Builds are made on Ubuntu 22.04, so use a distribution at least that recent.
- **macOS, Apple Silicon.** Any M1 or newer, on macOS 11 or later.
- **Windows, x64.** Vulkan or DirectX 12. Horizon picks DirectX 12 by itself when Vulkan does not work.

There is no build for Intel Macs, 32-bit systems, or ARM Linux and Windows. The server can be built from source on other systems.

## Where to go next

- **Playing:** [install and first launch](/docs/install), [settings worth knowing](/docs/settings), [multiplayer](/docs/multiplayer) and [what to do when something breaks](/docs/troubleshooting).
- **Hosting:** [host a server](/docs/hosting), from the launcher or the command line.
- **Going further:** [modpacks](/docs/modpacks), [mods and data packs](/docs/mods) and [performance](/docs/performance).
- **Making mods:** [make a mod](/docs/make-a-mod), from the first line of code to the registry, and the [Mod API](/docs/api/mods).
- **Helping out:** [contribute](/docs/contributing) and the [FAQ](/docs/faq).
