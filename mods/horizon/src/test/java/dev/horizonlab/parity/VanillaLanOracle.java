package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.net.InetAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HexFormat;
import java.util.List;
import net.minecraft.client.server.LanServer;
import net.minecraft.client.server.LanServerDetection;
import net.minecraft.client.server.LanServerPinger;

/**
 * Vanilla's LAN world discovery for tools/test_lan_parity.py. Each input
 * line is a case: `ping MOTD PORT` prints the announcement
 * LanServerPinger sends (its UTF-8 bytes); `heard IP HEX` decodes a received
 * datagram as LanServerDetection does and adds it to the list
 * (LanServerList.addServer), printing what it parsed and the list when it
 * changed; `reset` starts a new list (a new multiplayer screen).
 */
public final class VanillaLanOracle {
    public static void main(String[] args) throws Exception {
        List<String> cases = Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8);
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            LanServerDetection.LanServerList list = new LanServerDetection.LanServerList();
            for (String line : cases) {
                String[] f = line.split("\t", -1);
                switch (f[0]) {
                    case "ping" -> {
                        String ping = LanServerPinger.createPingString(f[1], f[2]);
                        out.println("ping " + HexFormat.of().formatHex(ping.getBytes(StandardCharsets.UTF_8)));
                    }
                    case "heard" -> {
                        byte[] bytes = HexFormat.of().parseHex(f[2]);
                        String received = new String(bytes, 0, bytes.length, StandardCharsets.UTF_8);
                        out.println("heard motd=" + LanServerPinger.parseMotd(received) + " address=" + LanServerPinger.parseAddress(received));
                        list.addServer(received, InetAddress.getByName(f[1]));
                        List<LanServer> dirty = list.takeDirtyServers();
                        if (dirty != null) {
                            StringBuilder b = new StringBuilder("list");
                            for (LanServer s : dirty) b.append(" [").append(s.getMotd()).append(" @ ").append(s.getAddress()).append(']');
                            out.println(b);
                        }
                    }
                    case "reset" -> list = new LanServerDetection.LanServerList();
                    default -> throw new IllegalArgumentException(line);
                }
            }
        }
    }
}
