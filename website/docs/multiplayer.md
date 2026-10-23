# Multiplayer

You can play with friends from your own world, on your local network, or on a server. This page also says plainly what does not work yet: servers that check Minecraft accounts.

## Play with friends from your own world

This needs no server and no account.

1. In your world, open the pause menu and choose the **Friends** button (the icon with two people), then **Start Sharing**. Horizon opens the world to your network and shows an **invite code** with **Copy Code**.
2. Send the code to your friends.
3. On their side, your friends choose the **Friends** button on the title screen (or **Join a friend** on the launcher's **Play** page), paste the code and choose **Join**. Pasting the code in **Direct Connection** works too.

There is no Horizon server in the middle. The code holds the host's addresses and a secret key, and the game between you is encrypted with it. A code works for as long as you keep sharing: closing the world or quitting ends it.

**Can your friends reach you?** The Play with Friends screen says what the game found:

- On the same network, or over a VPN such as Tailscale or ZeroTier, it just works.
- Over the internet, it asks your router to open the port by itself (UPnP). If the router does not allow it, forward TCP port 25580 to your computer.
- Some internet providers share one address between many homes (CGNAT), and no port can be opened. Use a VPN such as [Tailscale](https://tailscale.com) instead; the code already contains your VPN address.

Guests join with the name their game uses. There is no account check, so only share the code with people you trust.

Guests can also use the host's workstations: the crafter, loom, stonecutter, grindstone, anvil, smithing table, enchanting table, beacon, lectern and cartography table.

## Open to LAN

As in vanilla, **Open to LAN** in the pause menu lets people on the same network join your world. They see it in their server list. A vanilla 26.3 client can join a Horizon LAN world, and Horizon can join a vanilla one. LAN worlds are in offline mode.

## Joining a server

The Multiplayer page works like vanilla's: **Add server** saves one, **Direct connect** takes an address, and your servers are listed with their message of the day and ping. Horizon joins vanilla 26.3 servers and Horizon's own.

**Online mode.** Most public servers run in online mode: they check that each player really owns Minecraft by asking Mojang. To pass that check, the game needs a Microsoft account sign-in, and Horizon cannot do that yet. It needs an app registration and an approval from Mojang that the project does not have. So today:

- **Horizon can join servers in offline mode only** (`online-mode=false`): your own, a friend's, or a server on your network or a VPN.
- **It cannot join servers that check accounts.** That is most public servers, and Realms.
- Skins served over https show the default skin unless they were already cached.

Anyone can join an offline server under any name, so run one only on your network or a VPN, or share it with people you trust. [Host a server](/docs/hosting) explains the details.

## Vanilla players on a Horizon server

A vanilla 26.3 client can join a Horizon server, and in online mode it signs in with its own account as on a vanilla server. Secure chat works for those players.
