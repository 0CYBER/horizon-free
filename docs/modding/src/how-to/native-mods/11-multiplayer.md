# Multiplayer

- **Single player and LAN host:** the mods run in the host's game and its integrated server, so a server-side
  hook applies to every player of the world.
- **Joining a server:** client-side hooks run locally. The configuration handshake requires matching both-side mod IDs
  and versions; a refusal names the missing, extra or different-version mods.
- **Dedicated server:** native mods run in a standalone engine export built with `--server-output`, not in an
  instance build (`--server-output` conflicts with `--instance` and needs an explicit mod `FOLDER`). See
  [Standalone server export](12-standalone-server-export.md).
