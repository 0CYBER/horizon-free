# Logs, crashes and bug reports

When something breaks, Horizon writes down what happened. Send us that and the bug is much easier to fix. This page says where it is and how to report it.

## If the game crashes

The launcher notices. It shows what went wrong, in one line, with buttons:

- **Open report** opens the crash report.
- **Copy report** copies it, so you can paste it where you report the bug.
- **Open logs folder** opens the folder with the logs and crash reports.
- **Report on Discord** takes you to our Discord server.

## Where the files are

Everything is in the game folder, which is the same for every version you install:

- Linux: `~/.local/share/horizon`
- macOS: `~/Library/Application Support/horizon`
- Windows: `%APPDATA%\horizon`

Inside it:

- `logs/latest.log` is the log of the current or last run. Older runs are kept as `logs/<date>-<number>.log.gz`.
- `crash-reports/crash-<date>_<time>-client.txt` is written when the game crashes. It holds the error, where it happened, your computer, your graphics card, your render settings, your resource packs and the last 50 lines of the log.

The launcher's **Settings** page has a **Logs** row with an **Open** button. On the command line, `horizon-launcher logs --crash` prints the newest crash report and `horizon-launcher logs --lines 200` the end of the log.

## What to try first

- **The game does not start, or crashes straight away.** Update the launcher and Horizon, and your graphics drivers. On Windows, try **Graphics API: DirectX 12** (see [Settings](/docs/settings#graphics-api-on-windows)).
- **"Not prepared" or "Minecraft 26.3 was not found".** Play 26.3 once in the official Minecraft launcher, or point the launcher at your Minecraft folder (**Choose…** next to Minecraft folder on the Play page), then choose **Prepare** again.
- **Preparation says Java 25 is needed.** Install a Java 25 JDK, for example Eclipse Temurin 25.
- **Slow or stuttering.** Lower the render distance, turn LOD off, or pick the **Performance** preset. The performance overlay (Video, Advanced, Overlay) shows what is slow.
- **A resource or shader pack broke something.** Switch it off, or take it out of its folder.

## Reporting a bug

Good places, in this order:

1. **GitHub issues:** [github.com/ludovic111/horizon/issues](https://github.com/ludovic111/horizon/issues). Search first; someone may have reported it already.
2. **Discord:** the bugs channel on [our Discord server](https://discord.gg/HcTeMZxe5E), if you would rather talk it through.

Include:

- what you did and what you expected;
- your Horizon version (shown on the launcher's Play page) and the launcher's version;
- your system and graphics card;
- the crash report, or the end of `latest.log`, if there is one. Read it first: logs can hold your player name and server addresses, and lines of chat.

A Horizon bug is any place where it differs from vanilla 26.3, so "vanilla does X and Horizon does Y" is the best report there is.

## Data the launcher can send

The launcher can send performance statistics, crash reports, bug reports and benchmarks, but only the kinds you allow, and everything is off until you say yes. The game also sends an anonymous "I'm playing" ping while it runs, for the live player count. See [Privacy](/privacy) for what is sent, how long it is kept and how to delete it.
