package dev.horizonlab.parity;

import net.minecraft.client.Minecraft;
import net.minecraft.client.server.IntegratedServer;
import net.minecraft.server.MinecraftServer;

/**
 * The vanilla client as the reference LAN host for the multiplayer harness
 * (tools/test_lan_vanilla_client.py): it runs net.minecraft.client.main.Main
 * unchanged, and once the singleplayer world is up it turns off the
 * integrated server's authentication (it always authenticates guests against
 * Mojang's session servers, which offline test clients cannot pass) and opens
 * the world to LAN on the port in -Dhorizon.lanPort, with commands allowed,
 * as the "Open to LAN" screen does.
 */
public final class OfflineLanHost {
    public static void main(String[] args) throws Exception {
        int port = Integer.getInteger("horizon.lanPort", 0);
        Thread watcher = new Thread(() -> {
            boolean published = false;
            while (!published) {
                try {
                    Thread.sleep(500);
                    Minecraft mc = Minecraft.getInstance();
                    IntegratedServer server = mc == null ? null : mc.getSingleplayerServer();
                    if (server == null || mc.player == null || !server.isReady()) continue;
                    server.setUsesAuthentication(false);
                    mc.execute(() -> {
                        boolean ok = server.publishServer(MinecraftServer.MultiplayerScope.LAN, true, port);
                        System.out.println("[OfflineLanHost] published=" + ok + " port=" + server.getPort());
                    });
                    published = true;
                } catch (Throwable t) {
                    // The game is still starting.
                }
            }
        }, "horizon-offline-lan");
        watcher.setDaemon(true);
        watcher.start();
        net.minecraft.client.main.Main.main(args);
    }
}
