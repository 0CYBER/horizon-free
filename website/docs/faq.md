# FAQ

Short answers, with a link when there is more to say.

## Do I need to buy Minecraft?

Yes. Horizon contains no Minecraft code, assets or data. It builds its game files from your own copy of Minecraft: Java Edition 26.3, on your computer. See [Install](/docs/install).

## Is Horizon official?

No. It is not an official Minecraft product, and it is not approved by or associated with Mojang or Microsoft. It is open source, under the MIT licence.

## Will my Forge, NeoForge or Fabric mods work?

No. Java mod jars do not run in Horizon. Horizon has its own mod system, and the launcher can carry over the data-and-assets part of some Java mods. See [Modpacks](/docs/modpacks) and [Mods and data packs](/docs/mods).

## Can I join public servers?

Not yet. Most public servers check that you own Minecraft, which needs a Microsoft account sign-in, and Horizon cannot do that yet. It can join servers in offline mode, such as your own or a friend's. See [Multiplayer](/docs/multiplayer).

## When will Microsoft sign-in work?

We cannot give a date. It needs an app registration with Microsoft and an approval from Mojang that the project does not have yet. Until then, this site only describes what works today.

## Can vanilla players join my Horizon world or server?

Yes. A vanilla 26.3 client can join a Horizon server and a Horizon LAN world. See [Multiplayer](/docs/multiplayer) and [Host a server](/docs/hosting).

## Can I use my existing worlds?

Yes. Worlds from Minecraft 1.8 to 26.2 are upgraded the way vanilla upgrades them, and saves use vanilla's format. Copy your world into the game folder's `saves` folder, and make a backup first, as you would before opening a world in any new version.

## Where are my worlds and settings?

In the game folder: `~/.local/share/horizon` on Linux, `~/Library/Application Support/horizon` on macOS and `%APPDATA%\horizon` on Windows. Updating the game never touches it.

## Does it change my normal Minecraft install?

No. Horizon only reads your Minecraft files when it prepares, and keeps what it needs in its own folder.

## Why does preparing need Java 25?

Preparation runs vanilla's own data generator on your copy of the game, to read data such as every item's default components, and that is Java. It is needed for the preparation only. The game and the server themselves run without Java.

## What does "100% vanilla parity" mean?

Every subsystem matches vanilla in every harness that compares it. It means 100% of what the harnesses compare, not proof that nothing else differs. See [What Horizon is](/docs/) and [the status table](/#parity).

## Which systems does it run on?

Linux x86-64, macOS on Apple Silicon and Windows x64. See [supported systems](/docs/#supported-systems).

## Are shader packs supported?

In part. Horizon runs the full-screen effects of Java shader packs, not their geometry and shadow passes yet. See [Settings](/docs/settings#shader-packs).

## Does Horizon collect data?

Only if you say yes, and each kind has its own switch. Everything is off by default. The one thing sent without asking is an anonymous "I'm playing" ping, for the count of how many play right now: it holds a random number made each time the game starts, the version, your OS and whether a world is open to LAN, and nothing about you. See [Privacy](/privacy).

## How often does it update?

Often. Every change players can see is released automatically after it is merged, so new versions can come out several times a day. Each one is tested before it goes out (the game has to start and draw a world), and the launcher installs it as soon as it is out. There is one version for everyone. See [Install](/docs/install#updating).

## What do the version numbers mean?

Since 0.2, every release has a number: 0.2.0, then 0.2.1, 0.2.2 and so on, one for each release. The game and the launcher are both on 0.2 and count on their own, so you can play Horizon 0.2.7 with launcher 0.2.3. A bigger step, such as 0.3, is announced when it comes. Releases before 0.2 were named after the day they came out (2026.10.19.15); every 0.2 release is newer than all of them.

## How do I report a bug?

See [Logs, crashes and bug reports](/docs/troubleshooting).

## How can I help?

See [Contribute](/docs/contributing). You can also [sponsor the project on GitHub](https://github.com/sponsors/ludovic111).
