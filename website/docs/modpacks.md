# Modpacks

Java mod jars do not run in Horizon. Instead, the launcher reads a modpack, tells you which of its mods Horizon can carry over, and brings over the ones it can. This page explains what that means today and where the limits are.

> **Early.** The Modpacks page works, but no large pack runs as a whole in Horizon yet. What carries over today is mods made of data and assets: blocks, items, recipes, loot tables, tags, models, textures and structures. Mods with Java code do not, yet.

## What the Modpacks page does

Open **Modpacks** in the launcher. (From the next launcher release, it is the **Modpacks** tab of the **Mods** page.)

1. **Import a pack.** Choose **Import a pack file…** for a Modrinth `.mrpack` or a CurseForge zip, or **Import an instance folder…** for a Prism, MultiMC or CurseForge instance. The **Mods** page can also find a Modrinth modpack and import it for you (see [Mods and data packs](/docs/mods#finding-resource-packs-and-modpacks)). From the next launcher release it also ports it in the same step, so you can press **Play** right away.
2. **Read the coverage report.** For every mod in the pack it says **Ported**, **Partial**, **Not yet** or **Not needed**, with the reason. You can filter the list by status.
3. **Port.** **Port N mods** gets the jars of the mods this computer can carry over today, and carries them over for the version you picked. It copies the jars from a folder you have, or downloads them from the pack's own links (or the same file from Modrinth's download network), and checks their hashes. CurseForge downloads are coming soon, so for files that only CurseForge has, choose **Use jars from my instance…** and point it at your installed instance.
4. **Play.** **Play with N mods** starts the game as the pack's own profile, with its own saves and options, and those mods on.

Nothing is uploaded or shared. The mod jars and what is made from them stay on your computer.

## What carries over, and what does not

**Carries over today:** content mods whose substance is data and assets. Blocks and items built on vanilla's shapes, recipes, loot tables, tags, models, textures and language files, and structures that are described in data, such as the structure mods that use jigsaw pools. Loot tables and structures from the mods you carry show up in your world.

**Not yet:**

- **Mods with Java code:** machines, magic, new mob behaviour, anything that needs the mod's own logic. Each of these must be ported by hand to Horizon's mod system (Rust compiled to WebAssembly), and checked against the original mod running in a Java test instance. That work is under way; none of it is something you can install yet.
- **World generation written in code:** custom biomes, terrain shaping and structure types that are Java classes.
- **Scripts** such as KubeJS, and recipe viewers and map mods that are built around Java.

**Not needed:** performance mods such as Sodium, Lithium or FerriteCore (Horizon is the replacement), and libraries whose job is to give mods a common API. The report says which is which.

So a pack is a list of mods, and today Horizon can bring over a part of that list. The coverage report is the honest answer for your pack.

If two ported mods use the same namespace, the game can only load one of them. The launcher keeps the first, leaves the other off, and lists it under the pack.

Measured on three packs: Vanilla Perfected (108 files) ports 2 mods, Slimes Adventure (105) ports 20, and All the Mods 10 (496) ports 42. Each opens a world, with no crash. Every ported mod is partial: its blocks, items, recipes and structures come over, its Java code doesn't. A world with All the Mods 10's 42 ported mods takes about 40 seconds to load, against about 5 without mods.

## Example: All the Mods 10

All the Mods 10 has 495 jars. Horizon's report sorts them: about 290 add content, 80 are libraries, and the rest are integrations, interface mods, tools and performance mods. A few dozen of the content mods carry over today. The measured numbers are in [docs/modpacks.md](https://github.com/ludovic111/horizon/blob/main/docs/modpacks.md).

## From the command line

```text
horizon-launcher modpack list
horizon-launcher modpack import <pack>
horizon-launcher modpack port <pack> [--from <dir>]
horizon-launcher modpack play <pack>
horizon-launcher modpack remove <pack>
```

## Licences

Mods have their own licences, and Horizon respects them: it does not redistribute anyone's jars. The ports are made on your computer, from jars you downloaded, and stay there.
