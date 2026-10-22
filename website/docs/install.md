# Install and first launch

Download the launcher, let it install Horizon, then prepare the game from your own copy of Minecraft. It takes a few minutes, and you do the preparation once.

## What you need

- **Minecraft: Java Edition 26.3, played once.** Install it with the official Minecraft launcher (or Prism Launcher, MultiMC, the Modrinth App, ATLauncher or GDLauncher) and start 26.3 once, so it downloads its files. Horizon reads them from there and never copies them anywhere else.
- **Java 25, a JDK.** For example [Eclipse Temurin 25](https://adoptium.net/temurin/releases/?version=25). Only the one-time preparation uses it. The game itself needs no Java.
- **A computer on the [supported list](/docs/#supported-systems).** On Linux and Windows that means a graphics card with Vulkan (or DirectX 12 on Windows).

## Download the launcher

Get the newest launcher from the [Download button on the home page](/), which picks the right file for your system, or from [the launcher releases on GitHub](https://github.com/ludovic111/horizon/releases?q=launcher-v&expanded=true).

- **Linux:** download `horizon-launcher-linux-x86_64.tar.gz`, extract it and run `horizon-launcher`.
- **macOS:** download `horizon-launcher-macos-arm64.dmg` (Apple Silicon: M1 and later; Intel Macs: `horizon-launcher-macos-x64.dmg`, from the next release on), open it and drag Horizon onto Applications. The first time, macOS says it cannot check the app, because it is not signed with a paid Apple developer account. On macOS 14 and earlier, right-click Horizon, choose **Open**, then **Open** again. On macOS 15 and later, open it once, then go to **System Settings, Privacy & Security** and choose **Open Anyway**. After that it opens normally.
- **Windows:** download `horizon-launcher-windows-x64.zip`, extract it and run `horizon-launcher.exe`.

Every release also carries `SHA256SUMS` and a signature, `SHA256SUMS.sig`, if you want to check your download. The launcher does not check that signature for you yet.

## First launch

1. Open the launcher. On the **Play** page, if it says "No version installed yet", choose **Choose a version**. On the **Versions** page, install the newest one. The launcher installs it in its own folder.
2. Back on **Play**, the launcher says "Not prepared. Horizon needs the files of your own Minecraft 26.3." Choose **Prepare**. It finds your Minecraft folder by itself in the usual places. If it cannot, it lists where it looked, and **Choose…** next to the Minecraft folder lets you point it at yours.
3. Wait a few minutes. When it says "Prepared for Minecraft 26.3", choose **Play**.

The game opens on Horizon's own title screen. Your worlds, options and logs live in the game folder, which is not inside a version's folder, so updating or removing a version never touches them:

- Linux: `~/.local/share/horizon`
- macOS: `~/Library/Application Support/horizon`
- Windows: `%APPDATA%\horizon`

> **No Microsoft sign-in yet.** The launcher's Settings page has an Accounts section, but signing in with a Microsoft account does not work for players today. It needs an app registration and an approval from Mojang that the project does not have yet. You do not need an account to play single player.

## Updating

- **Horizon.** The launcher looks for a new release when it starts and every four hours while it is open. With **Install new versions automatically** on (the default), it installs one and moves you to it if you were on the newest. New releases come out many times a day, and old versions stay installed until you remove them on the **Versions** page.
- **The launcher.** It updates itself from its own releases, then shows **Restart** in its footer.

## Stable and beta builds

The launcher installs **stable** builds by default. In **Settings**, **Beta builds** installs new builds the moment they are released, before they have had time to settle. The **Versions** page shows each version's channel. A new release starts as a beta and becomes stable by itself about six hours later, once its smoke test has passed.

## The Versions page

It lists Horizon's releases, newest first. You can install a version, remove it, or choose which one **Play** starts. Each release's notes are on the **Patch notes** page, and on [GitHub](https://github.com/ludovic111/horizon/releases).

## Something went wrong

See [Logs, crashes and bug reports](/docs/troubleshooting).
