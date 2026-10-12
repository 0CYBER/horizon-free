package dev.horizonlab.parity;

import java.io.File;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Date;
import java.util.List;
import java.util.UUID;
import java.util.stream.Stream;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.Eula;
import net.minecraft.server.dedicated.DedicatedServerProperties;
import net.minecraft.server.notifications.NotificationManager;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.permissions.PermissionLevel;
import net.minecraft.server.players.IpBanList;
import net.minecraft.server.players.IpBanListEntry;
import net.minecraft.server.players.NameAndId;
import net.minecraft.server.players.ServerOpList;
import net.minecraft.server.players.ServerOpListEntry;
import net.minecraft.server.players.StoredUserList;
import net.minecraft.server.players.UserBanList;
import net.minecraft.server.players.UserBanListEntry;
import net.minecraft.server.players.UserWhiteList;
import net.minecraft.server.players.UserWhiteListEntry;

/**
 * The dedicated server's files read and written by vanilla's own classes:
 * server.properties through DedicatedServerProperties (every parsed value
 * and the file stored back), eula.txt through Eula, and the four user lists
 * through StoredUserList (load, scripted add/remove, save). Each case is a
 * directory under args[0]; the output (args[1]) has one "== case" section
 * per case. The random defaults (management secret, seed) and the stored
 * timestamp are printed as placeholders. Run with -Duser.timezone=UTC.
 */
public final class VanillaServerFilesOracle {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Path root = Path.of(args[0]);
        List<Path> cases;
        try (Stream<Path> s = Files.list(root)) {
            cases = s.filter(Files::isDirectory).sorted().toList();
        }
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            for (Path dir : cases) {
                out.println("== " + dir.getFileName());
                runCase(dir, out);
            }
        }
    }

    private static Path work(Path dir) throws Exception {
        Path work = Files.createTempDirectory("server-files");
        try (Stream<Path> s = Files.list(dir)) {
            for (Path f : s.toList()) Files.copy(f, work.resolve(f.getFileName()));
        }
        return work;
    }

    private static void runCase(Path dir, PrintWriter out) throws Exception {
        Path w = work(dir);
        Path props = w.resolve("server.properties");
        if (Files.exists(dir.resolve("server.properties")) || Files.exists(dir.resolve("properties.default"))) {
            String before = Files.exists(props) ? Files.readString(props, StandardCharsets.ISO_8859_1) : "";
            try {
                DedicatedServerProperties p = DedicatedServerProperties.fromFile(props);
                printProperties(p, before, out);
                p.store(props);
                out.println("stored:");
                List<String> lines = Files.readAllLines(props, StandardCharsets.UTF_8);
                for (int i = 0; i < lines.size(); i++) {
                    String line = lines.get(i);
                    if (i == 1 && line.startsWith("#")) line = "#DATE";
                    if (line.startsWith("management-server-secret=") && !hasKey(before, "management-server-secret")) {
                        line = "management-server-secret=<random>";
                    }
                    out.println(line);
                }
            } catch (Throwable t) {
                out.println("ERROR " + t.getClass().getName());
            }
        }
        if (Files.exists(dir.resolve("eula.txt")) || Files.exists(dir.resolve("eula.missing"))) {
            Path eula = w.resolve("eula.txt");
            boolean agreed = new Eula(eula).hasAgreedToEULA();
            out.println("eula agreed=" + agreed);
            if (Files.exists(eula)) {
                List<String> lines = Files.readAllLines(eula, StandardCharsets.ISO_8859_1);
                for (int i = 0; i < lines.size(); i++) {
                    out.println(i == 1 && lines.get(i).startsWith("#") ? "#DATE" : lines.get(i));
                }
            }
        }
        NotificationManager n = new NotificationManager();
        ServerOpList ops = new ServerOpList(w.resolve("ops.json").toFile(), n);
        UserWhiteList white = new UserWhiteList(w.resolve("whitelist.json").toFile(), n);
        UserBanList bans = new UserBanList(w.resolve("banned-players.json").toFile(), n);
        IpBanList ips = new IpBanList(w.resolve("banned-ips.json").toFile(), n);
        StoredUserList<?, ?>[] lists = {ops, white, bans, ips};
        String[] names = {"ops", "whitelist", "banned-players", "banned-ips"};
        boolean any = false;
        for (int i = 0; i < lists.length; i++) {
            if (Files.exists(lists[i].getFile().toPath())) {
                any = true;
                try {
                    lists[i].load();
                } catch (Throwable t) {
                    out.println(names[i] + " load ERROR " + t.getClass().getName());
                }
            }
        }
        Path actions = dir.resolve("actions.txt");
        if (Files.exists(actions)) {
            any = true;
            for (String line : Files.readAllLines(actions, StandardCharsets.UTF_8)) {
                if (line.isBlank()) continue;
                String[] a = line.split("\t", -1);
                boolean r = switch (a[0]) {
                    case "op" -> ops.add(new ServerOpListEntry(user(a[1], a[2]),
                        LevelBasedPermissionSet.forLevel(PermissionLevel.byId(Integer.parseInt(a[3]))), Boolean.parseBoolean(a[4])));
                    case "deop" -> ops.remove(user(a[1], a[2]));
                    case "white" -> white.add(new UserWhiteListEntry(user(a[1], a[2])));
                    case "unwhite" -> white.remove(user(a[1], a[2]));
                    case "ban" -> bans.add(new UserBanListEntry(user(a[1], a[2]), date(a[3]), opt(a[4]), date(a[5]), opt(a[6])));
                    case "pardon" -> bans.remove(user(a[1], a[2]));
                    case "banip" -> ips.add(new IpBanListEntry(a[1], date(a[2]), opt(a[3]), date(a[4]), opt(a[5])));
                    case "pardonip" -> ips.remove(a[1]);
                    case "isbanned" -> bans.isBanned(user(a[1], a[2]));
                    case "isop" -> ops.get(user(a[1], a[2])) != null;
                    default -> throw new IllegalArgumentException(a[0]);
                };
                out.println("action " + a[0] + " " + r);
            }
        }
        if (any) {
            for (int i = 0; i < lists.length; i++) {
                lists[i].save();
                out.println(names[i] + ":");
                out.println(Files.readString(lists[i].getFile().toPath(), StandardCharsets.UTF_8));
            }
            out.println("op users " + String.join(",", sorted(ops.getUserList())));
            out.println("white users " + String.join(",", sorted(white.getUserList())));
        }
    }

    private static List<String> sorted(String[] names) {
        List<String> l = new ArrayList<>(List.of(names));
        l.sort(null);
        return l;
    }
    private static NameAndId user(String name, String uuid) {
        return new NameAndId(UUID.fromString(uuid), name);
    }
    private static Date date(String s) {
        return s.equals("-") ? null : new Date(Long.parseLong(s));
    }
    private static String opt(String s) {
        return s.equals("-") ? null : s;
    }
    private static boolean hasKey(String text, String key) {
        for (String line : text.split("\n")) {
            if (line.trim().startsWith(key)) return true;
        }
        return false;
    }

    private static void printProperties(DedicatedServerProperties p, String before, PrintWriter out) {
        // The fields in declaration order.
        out.println("online-mode=" + p.onlineMode);
        out.println("prevent-proxy-connections=" + p.preventProxyConnections);
        out.println("server-ip=" + p.serverIp);
        out.println("allow-flight=" + p.allowFlight.get());
        out.println("motd=" + p.motd.get());
        out.println("enable-code-of-conduct=" + p.codeOfConduct);
        out.println("bug-report-link=" + p.bugReportLink);
        out.println("force-gamemode=" + p.forceGameMode.get());
        out.println("enforce-whitelist=" + p.enforceWhitelist.get());
        out.println("difficulty=" + p.difficulty.get().getSerializedName());
        out.println("gamemode=" + p.gameMode.get().getName());
        out.println("level-name=" + p.levelName);
        out.println("server-port=" + p.serverPort);
        out.println("management-server-enabled=" + p.managementServerEnabled);
        out.println("management-server-host=" + p.managementServerHost);
        out.println("management-server-port=" + p.managementServerPort);
        out.println("management-server-secret=" + (hasKey(before, "management-server-secret") ? p.managementServerSecret : "<random>"));
        out.println("management-server-tls-enabled=" + p.managementServerTlsEnabled);
        out.println("management-server-tls-keystore=" + p.managementServerTlsKeystore);
        out.println("management-server-tls-keystore-password=" + p.managementServerTlsKeystorePassword);
        out.println("management-server-allowed-origins=" + p.managementServerAllowedOrigins);
        out.println("announce-player-achievements=" + p.announcePlayerAchievements);
        out.println("enable-query=" + p.enableQuery);
        out.println("query.port=" + p.queryPort);
        out.println("enable-rcon=" + p.enableRcon);
        out.println("rcon.port=" + p.rconPort);
        out.println("rcon.password=" + p.rconPassword);
        out.println("hardcore=" + p.hardcore);
        out.println("use-native-transport=" + p.useNativeTransport);
        out.println("spawn-protection=" + p.spawnProtection.get());
        out.println("op-permission-level=" + p.opPermissions.get().level().id());
        out.println("function-permission-level=" + p.functionPermissions.level().id());
        out.println("max-tick-time=" + p.maxTickTime);
        out.println("max-chained-neighbor-updates=" + p.maxChainedNeighborUpdates);
        out.println("rate-limit=" + p.rateLimitPacketsPerSecond);
        out.println("command-spam-threshold-seconds=" + p.commandSpamThresholdSeconds);
        out.println("chat-spam-threshold-seconds=" + p.chatSpamThresholdSeconds);
        out.println("view-distance=" + p.viewDistance.get());
        out.println("simulation-distance=" + p.simulationDistance.get());
        out.println("max-players=" + p.maxPlayers.get());
        out.println("network-compression-threshold=" + p.networkCompressionThreshold);
        out.println("broadcast-rcon-to-ops=" + p.broadcastRconToOps);
        out.println("broadcast-console-to-ops=" + p.broadcastConsoleToOps);
        out.println("max-world-size=" + p.maxWorldSize);
        out.println("sync-chunk-writes=" + p.syncChunkWrites);
        out.println("region-file-compression=" + p.regionFileComression);
        out.println("enable-jmx-monitoring=" + p.enableJmxMonitoring);
        out.println("enable-status=" + p.enableStatus.get());
        out.println("hide-online-players=" + p.hideOnlinePlayers.get());
        out.println("entity-broadcast-range-percentage=" + p.entityBroadcastRangePercentage.get());
        out.println("text-filtering-config=" + p.textFilteringConfig);
        out.println("text-filtering-version=" + p.textFilteringVersion);
        out.println("player-idle-timeout=" + p.playerIdleTimeout.get());
        out.println("status-heartbeat-interval=" + p.statusHeartbeatInterval.get());
        out.println("white-list=" + p.whiteList.get());
        out.println("enforce-secure-profile=" + p.enforceSecureProfile);
        out.println("log-ips=" + p.logIPs);
        out.println("pause-when-empty-seconds=" + p.pauseWhenEmptySeconds.get());
        out.println("accepts-transfers=" + p.acceptsTransfers.get());
        String levelSeed;
        try {
            java.lang.reflect.Field f = net.minecraft.server.dedicated.Settings.class.getDeclaredField("properties");
            f.setAccessible(true);
            levelSeed = ((java.util.Properties) f.get(p)).getProperty("level-seed", "").trim();
        } catch (ReflectiveOperationException e) {
            throw new RuntimeException(e);
        }
        out.println("seed=" + (levelSeed.isEmpty() ? "<random>" : Long.toString(p.worldOptions.seed())));
        out.println("generate-structures=" + p.worldOptions.generateStructures());
        out.println("resource-pack=" + p.serverResourcePackInfo.map(i ->
            i.id() + " " + i.url() + " " + i.hash() + " " + i.isRequired() + " " + (i.prompt() != null)).orElse("none"));
        out.println("initial-enabled-packs=" + String.join("|", p.initialDataPackConfiguration.getEnabled()));
        out.println("initial-disabled-packs=" + String.join("|", p.initialDataPackConfiguration.getDisabled()));
    }
}
