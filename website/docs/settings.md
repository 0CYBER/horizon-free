# Settings worth knowing

Horizon plays like vanilla out of the box. This page covers the settings that are different or new: graphics, distant terrain, resource packs, controllers and a few optional extras.

The game's menus are vanilla's. Horizon adds its own settings to vanilla's **Options** screen the way mods do: **Horizon Settings...** and **Mods...** take the last two buttons of the grid. **Horizon Settings** opens Graphics, Advanced Graphics, Controller Settings, Extras and Mods, each a screen of vanilla's buttons and sliders. The graphics buttons are also at the top of **Video Settings**, and Controller Settings at the bottom of **Controls**.

![Video Settings, with Horizon's graphics buttons at the top](/media/settings-800.webp)

## Graphics: presets, Simple and Advanced

**Options, Video Settings** starts with **Horizon Graphics...** and **Advanced Graphics...** (also in **Options, Horizon Settings**).

- **Horizon Graphics** has a preset (Performance, Balanced, Quality, Ultra or Custom) and LOD distance.
- **Advanced Graphics** has every Horizon setting, in groups: Rendering, Quality, Display, Preferences, Performance and Overlay.

Vanilla's own settings (render distance, field of view, frame rate, VSync, GUI scale and the rest) stay on Video Settings, where vanilla has them.

Every row has a one-line description and its cost in frame time: none, low, medium or high. Changes apply at once, except worker threads, which apply at the next start. A preset sets the quality settings together, and changing one of them turns the preset into **Custom**.

Two settings are worth knowing on laptops and handhelds. **Battery mode** (Performance group, off by default) caps the frame rate at 60 in a world and 30 in menus, and uses half the automatic worker threads. The **performance overlay** (Overlay group) shows the frame rate, frame times and a graph over the game.

Not available: anisotropic filtering, MSAA (FXAA is there instead) and exclusive fullscreen (fullscreen is borderless).

## Distant terrain (LOD)

**LOD Distance** draws far-away terrain as simplified columns, so you can see much farther than the render distance. It is **off by default**. Turn it on in the Video settings and choose 64, 128, 256, 512 or 1024 chunks.

- Keep the normal render distance modest next to it. For example 12 chunks with a LOD distance of 256. **Full Detail Distance** sets the nearby range.
- LOD does not raise the simulation distance: far terrain is only drawn, not simulated.
- Detail arrives gradually. A big radius does not mean everything is drawn right away.
- Far terrain is made from the world's seed, so places you have not visited are drawn too, close to what they will be. Small features, fluid edges and lighting can differ from the real chunks.
- Horizon caps far terrain on integrated graphics and holds it back on computers with four threads.

## Graphics API on Windows

Linux always uses Vulkan and macOS always uses Metal. On Windows there is a choice, in **Options, Video Settings, Advanced Graphics, Performance, Graphics API**:

- **Auto** (the default) uses Vulkan, and DirectX 12 when Vulkan has no graphics card or driver that works.
- **Vulkan** and **DirectX 12** force one of the two.

The change applies at the next start. If Vulkan gives you trouble, try DirectX 12.

## Shader packs

Shader packs have been removed for now. They will come back later, written again from scratch. Packs in your `shaderpacks` folder are left where they are.

## Resource packs

Java Edition resource packs work. Put the folder or `.zip` in the `resourcepacks` folder of the game folder and pick it on the **Resource Packs** screen, which is vanilla's screen. The game reloads every loader from the pack, as vanilla does.

The launcher's **Mods** page can download them from Modrinth (see [Mods and data packs](/docs/mods#finding-resource-packs-and-modpacks)).

[Resource packs: what works](https://github.com/ludovic111/horizon/blob/main/docs/resource-packs.md) lists everything a 26.3 resource pack can change and whether Horizon honours it.

## Controllers

Horizon plays with PlayStation (DualShock 4 and DualSense), Xbox (360, One and Series) and Nintendo Switch Pro controllers, and with the Steam Deck's built-in controls. Vanilla has no controller support at all, so this is Horizon's own. With no controller connected, nothing changes.

- Settings are in **Options, Controls, Controller Settings** (or **Options, Horizon Settings, Controller Settings**): on or off, vibration, look sensitivity, inverting the axes, deadzones, button glyphs and remapping.
- Menus, the inventory and chat work with a controller, with an on-screen keyboard for text.
- On a DualSense you can also set adaptive triggers and the lightbar.
- On the Steam Deck, Steam's default layout works with no setup.

## Extras

**Options, Horizon Settings, Extras** holds small, optional conveniences. Every one is off by default, so a fresh install behaves like vanilla: fullbright, a minimap, auto-reconnect, Mouse Tweaks-style inventory controls and inventory sorting. The FPS counter, the keystroke display and zoom are official mods now, in **Options, Mods**: if you had one on, the game installed it for you on its first start after the update, with your settings. One more is **dynamic lights**: a torch, lantern or other light source that you hold, that lies dropped as an item, or a burning mob, lights the blocks around it. It is client-side only, so other players do not see it.

## Where your settings are stored

In the game folder, next to `options.txt`. That file keeps vanilla's own keys and values, so a vanilla client can share the folder. Horizon's own settings are in `horizon-graphics.txt`, `horizon-controller.txt` and a few more beside it.
