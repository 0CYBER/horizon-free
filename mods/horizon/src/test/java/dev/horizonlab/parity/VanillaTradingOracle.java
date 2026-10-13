package dev.horizonlab.parity;

import com.mojang.authlib.GameProfile;
import com.mojang.serialization.Lifecycle;
import io.netty.channel.embedded.EmbeddedChannel;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.net.Proxy;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.TreeSet;
import java.util.UUID;
import java.util.function.BooleanSupplier;
import net.minecraft.SystemReport;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.gametest.framework.GameTestServer;
import net.minecraft.gizmos.GizmoCollector;
import net.minecraft.gizmos.Gizmos;
import net.minecraft.nbt.*;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.Services;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.WorldStem;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.level.progress.LoggingLevelLoadListener;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.server.notifications.EmptyNotificationService;
import net.minecraft.server.notifications.NotificationManager;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.permissions.PermissionSet;
import net.minecraft.server.players.NameAndId;
import net.minecraft.server.players.PlayerList;
import net.minecraft.util.ProblemReporter;
import net.minecraft.util.Util;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.util.debugchart.LocalSampleLogger;
import net.minecraft.util.debugchart.SampleLogger;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.RandomSequences;
import net.minecraft.world.clock.WorldClocks;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.ExperienceOrb;
import net.minecraft.world.entity.ai.gossip.GossipType;
import net.minecraft.world.entity.ai.village.ReputationEventType;
import net.minecraft.world.entity.npc.villager.AbstractVillager;
import net.minecraft.world.entity.npc.villager.Villager;
import net.minecraft.world.entity.npc.villager.VillagerData;
import net.minecraft.world.entity.npc.villager.VillagerProfession;
import net.minecraft.world.entity.npc.villager.VillagerType;
import net.minecraft.world.entity.npc.wanderingtrader.WanderingTrader;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.inventory.ContainerInput;
import net.minecraft.world.inventory.MerchantMenu;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.trading.MerchantOffer;
import net.minecraft.world.item.trading.MerchantOffers;
import net.minecraft.world.item.trading.TradeSet;
import net.minecraft.world.level.DataPackConfig;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.dimension.LevelStem;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.WorldDimensions;
import net.minecraft.world.level.levelgen.WorldGenSettings;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.storage.LevelDataAndDimensions;
import net.minecraft.world.level.storage.LevelStorageSource;
import net.minecraft.world.level.storage.PrimaryLevelData;
import net.minecraft.world.level.storage.TagValueOutput;
import net.minecraft.world.phys.AABB;

/**
 * Trading oracle: villagers and wandering traders on a real (headless, flat, structure-free)
 * vanilla server level, so exploration maps find nothing and are discarded.
 *
 * <p>Each case resets every trade set's random sequence to the case's world seed, then:
 * <ul>
 * <li>{@code gen}: villagers reach their levels one by one (updateTrades per level, as
 * increaseMerchantCareer does) and wandering traders get their offers, in order; prints each
 * one's Offers as saved, then the next int of every trade set sequence.</li>
 * <li>{@code price}: a villager's offers after Villager.updateSpecialPrices for a player with
 * gossip and Hero of the Village, with uses and demand set first.</li>
 * <li>{@code restock}: uses, then Villager.shouldRestock / restock at given game times and days.</li>
 * <li>{@code trade}: the player opens the trading screen (mobInteract) and selects offers,
 * clicks slots, ticks (the TRADE reputation event) and closes it; the menu, the villager and the
 * orbs it drops are printed after each step.</li>
 * <li>{@code nbt}: the villager as saved (SNBT), for the Rust side to read and write back.</li>
 * </ul>
 * NBT is printed in a canonical form: sorted compound keys, typed numbers, floats as raw bits.
 */
public final class VanillaTradingOracle {
    static final List<String[]> CASES = new ArrayList<>();
    static PrintWriter out;
    static Path UNIVERSE;
    static final UUID PLAYER_UUID = new UUID(0x1234L, 0x5678L);

    public static void main(String[] args) throws Exception {
        for (String line : Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8)) {
            if (!line.isBlank()) CASES.add(line.trim().split(" "));
        }
        out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
        Path universe = Path.of(args[2]);
        Files.createDirectories(universe);
        net.minecraft.SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Util.startTimerHackThread();
        UNIVERSE = universe;
        LevelStorageSource.LevelStorageAccess access = LevelStorageSource.createDefault(universe).createAccess("tradeworld");
        PackRepository packs = ServerPacksSource.createPackRepository(access);
        MinecraftServer.spin(thread -> Server.create(thread, access, packs));
    }

    static Field field(Class<?> c, String name) throws Exception {
        for (Class<?> k = c; k != null; k = k.getSuperclass()) {
            try {
                Field f = k.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException e) {
                // keep looking
            }
        }
        throw new NoSuchFieldException(name);
    }

    static Method method(Class<?> c, String name, Class<?>... types) throws Exception {
        for (Class<?> k = c; k != null; k = k.getSuperclass()) {
            try {
                Method m = k.getDeclaredMethod(name, types);
                m.setAccessible(true);
                return m;
            } catch (NoSuchMethodException e) {
                // keep looking
            }
        }
        throw new NoSuchMethodException(name);
    }

    /** Canonical NBT text, the same on both sides. */
    static void canon(Tag t, StringBuilder sb) {
        switch (t) {
            case CompoundTag c -> {
                sb.append('{');
                boolean first = true;
                for (String k : new TreeSet<>(c.keySet())) {
                    if (!first) sb.append(',');
                    first = false;
                    quote(k, sb);
                    sb.append(':');
                    canon(c.get(k), sb);
                }
                sb.append('}');
            }
            case ListTag l -> {
                sb.append('[');
                for (int i = 0; i < l.size(); i++) {
                    if (i > 0) sb.append(',');
                    canon(l.get(i), sb);
                }
                sb.append(']');
            }
            case StringTag s -> quote(s.value(), sb);
            case ByteTag b -> sb.append(b.value()).append('b');
            case ShortTag s -> sb.append(s.value()).append('s');
            case IntTag i -> sb.append(i.value());
            case LongTag l -> sb.append(l.value()).append('L');
            case FloatTag f -> sb.append("f:").append(Integer.toHexString(Float.floatToRawIntBits(f.value())));
            case DoubleTag d -> sb.append("d:").append(Long.toHexString(Double.doubleToRawLongBits(d.value())));
            case ByteArrayTag a -> {
                sb.append("[B;");
                byte[] v = a.getAsByteArray();
                for (int i = 0; i < v.length; i++) sb.append(i > 0 ? "," : "").append(v[i]);
                sb.append(']');
            }
            case IntArrayTag a -> {
                sb.append("[I;");
                int[] v = a.getAsIntArray();
                for (int i = 0; i < v.length; i++) sb.append(i > 0 ? "," : "").append(v[i]);
                sb.append(']');
            }
            case LongArrayTag a -> {
                sb.append("[L;");
                long[] v = a.getAsLongArray();
                for (int i = 0; i < v.length; i++) sb.append(i > 0 ? "," : "").append(v[i]);
                sb.append(']');
            }
            default -> sb.append("?").append(t.getClass().getSimpleName());
        }
    }

    static void quote(String s, StringBuilder sb) {
        sb.append('"');
        for (char c : s.toCharArray()) {
            if (c == '"' || c == '\\') sb.append('\\');
            sb.append(c);
        }
        sb.append('"');
    }

    static String canon(Tag t) {
        StringBuilder sb = new StringBuilder();
        canon(t, sb);
        return sb.toString();
    }

    static final class Server extends MinecraftServer {
        // A default world: the vanilla pack and features only (no trade rebalance).
        static final FeatureFlagSet FEATURES = FeatureFlags.DEFAULT_FLAGS;
        final LocalSampleLogger sampleLogger = new LocalSampleLogger(4);
        int phase = 0;
        ServerPlayer player;

        static Server create(Thread thread, LevelStorageSource.LevelStorageAccess access, PackRepository packs) {
            try {
                packs.reload();
                ArrayList<String> enabled = new ArrayList<>(List.of("vanilla"));
                WorldDataConfiguration config = new WorldDataConfiguration(new DataPackConfig(enabled, List.of()), FEATURES);
                LevelSettings settings = new LevelSettings("Trade Level", GameType.SURVIVAL, LevelSettings.DifficultySettings.DEFAULT, true, config);
                WorldLoader.PackConfig packConfig = new WorldLoader.PackConfig(packs, config, false, true);
                WorldLoader.InitConfig init = new WorldLoader.InitConfig(packConfig, Commands.CommandSelection.DEDICATED, LevelBasedPermissionSet.OWNER);
                WorldStem stem = Util.<WorldStem>blockUntilDone(executor -> WorldLoader.load(init, context -> {
                    Registry<LevelStem> none = new MappedRegistry<>(Registries.LEVEL_STEM, Lifecycle.stable()).freeze();
                    WorldDimensions dims = context.datapackWorldRegistries().lookupOrThrow(Registries.WORLD_PRESET)
                        .getOrThrow(WorldPresets.FLAT).value().createWorldDimensions();
                    WorldDimensions.Complete complete = dims.bake(none);
                    PrimaryLevelData data = new PrimaryLevelData(settings, complete.specialWorldProperty(), complete.lifecycle());
                    return new WorldLoader.DataLoadOutput<>(
                        new LevelDataAndDimensions.WorldDataAndGenSettings(data, new WorldGenSettings(new WorldOptions(0L, false, false), dims)),
                        complete.dimensionsRegistryAccess());
                }, WorldStem::new, Util.backgroundExecutor(), executor)).get();
                GameRules rules = new GameRules(FEATURES);
                rules.set(GameRules.SPAWN_MOBS, false, null);
                rules.set(GameRules.SPAWN_MONSTERS, false, null);
                rules.set(GameRules.SPAWN_PATROLS, false, null);
                rules.set(GameRules.SPAWN_PHANTOMS, false, null);
                rules.set(GameRules.SPAWN_WANDERING_TRADERS, false, null);
                rules.set(GameRules.SPAWN_WARDENS, false, null);
                rules.set(GameRules.ADVANCE_TIME, false, null);
                rules.set(GameRules.ADVANCE_WEATHER, false, null);
                rules.set(GameRules.RAIDS, false, null);
                Field nf = GameTestServer.class.getDeclaredField("NO_SERVICES");
                nf.setAccessible(true);
                return new Server(thread, access, packs, stem, rules, (Services) nf.get(null));
            } catch (Exception e) {
                e.printStackTrace();
                System.exit(2);
                throw new IllegalStateException(e);
            }
        }

        Server(Thread thread, LevelStorageSource.LevelStorageAccess access, PackRepository packs, WorldStem stem, GameRules rules, Services services) {
            super(thread, access, packs, stem, Optional.of(rules), Proxy.NO_PROXY, DataFixers.getDataFixer(), services,
                LoggingLevelLoadListener.forDedicatedServer(), false, new NotificationManager());
        }

        @Override
        protected boolean initServer() {
            this.setPlayerList(new PlayerList(this, this.registries(), this.playerDataStorage, new EmptyNotificationService()) {
                @Override
                public void broadcast(Player except, double x, double y, double z, double range, ResourceKey<Level> dim, Packet<?> packet) {
                }
            });
            Gizmos.withCollector(GizmoCollector.NOOP);
            this.loadLevel();
            return true;
        }

        @Override
        protected void tickServer(BooleanSupplier haveTime) {
            try {
                step(haveTime);
            } catch (Throwable t) {
                t.printStackTrace();
                out.flush();
                System.exit(3);
            }
        }

        void step(BooleanSupplier haveTime) throws Exception {
            ServerLevel level = this.overworld();
            if (phase == 0) {
                for (int dx = -1; dx <= 1; dx++)
                    for (int dz = -1; dz <= 1; dz++)
                        level.setChunkForced(dx, dz, true);
                phase = 1;
                super.tickServer(haveTime);
                return;
            }
            super.tickServer(haveTime);
            if (!level.isPositionEntityTicking(new BlockPos(0, -60, 0))) return;
            for (String[] c : CASES) {
                try {
                    run(c, level);
                } catch (Exception e) {
                    out.println(c[1] + " error " + e);
                    e.printStackTrace();
                }
            }
            out.close();
            try (var paths = Files.walk(UNIVERSE)) {
                paths.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            } catch (Exception e) {
                // best effort
            }
            Runtime.getRuntime().halt(0);
        }

        Map<String, String> kv(String[] c) {
            Map<String, String> m = new HashMap<>();
            for (int i = 2; i < c.length; i++) {
                int eq = c[i].indexOf('=');
                if (eq > 0) m.merge(c[i].substring(0, eq), c[i].substring(eq + 1), (a, b) -> a + " " + b);
            }
            return m;
        }

        void resetSequences(ServerLevel level, long seed) {
            RandomSequences rs = this.getRandomSequences();
            rs.clear();
            level.registryAccess().lookupOrThrow(Registries.TRADE_SET).listElements()
                .forEach(h -> h.value().randomSequence().ifPresent(id -> rs.reset(id, seed)));
        }

        ServerPlayer player(ServerLevel level) throws Exception {
            if (player == null) {
                CommonListenerCookie cookie = CommonListenerCookie.createInitial(new GameProfile(PLAYER_UUID, "horizon-trade"), false);
                player = new ServerPlayer(this, level, cookie.gameProfile(), cookie.clientInformation());
                Connection connection = new Connection(PacketFlow.SERVERBOUND);
                new EmbeddedChannel(connection);
                this.getPlayerList().placeNewPlayer(connection, player, cookie);
                method(player.connection.getClass(), "markClientLoaded").invoke(player.connection);
            }
            if (player.containerMenu != player.inventoryMenu) player.closeContainer();
            player.getInventory().clearContent();
            player.removeAllEffects();
            player.setGameMode(GameType.SURVIVAL);
            player.snapTo(1.5, -60.0, 0.5, 0.0F, 0.0F);
            return player;
        }

        /** A villager or wandering trader from spec PROF:TYPE:LEVEL or "wt", generated as it levels up. */
        AbstractVillager merchant(ServerLevel level, String spec, double[] at, Long randomSeed) throws Exception {
            AbstractVillager v;
            if (spec.equals("wt")) {
                v = net.minecraft.world.entity.EntityTypes.WANDERING_TRADER.create(level, EntitySpawnReason.COMMAND);
            } else {
                String[] p = spec.split(":");
                Villager villager = net.minecraft.world.entity.EntityTypes.VILLAGER.create(level, EntitySpawnReason.COMMAND);
                Holder<VillagerType> type = BuiltInRegistries.VILLAGER_TYPE.getOrThrow(
                    ResourceKey.create(Registries.VILLAGER_TYPE, Identifier.withDefaultNamespace(p[1])));
                Holder<VillagerProfession> prof = BuiltInRegistries.VILLAGER_PROFESSION.getOrThrow(
                    ResourceKey.create(Registries.VILLAGER_PROFESSION, Identifier.withDefaultNamespace(p[0])));
                villager.setVillagerData(new VillagerData(type, prof, 1));
                v = villager;
            }
            v.snapTo(at[0], at[1], at[2], 0.0F, 0.0F);
            if (randomSeed != null) field(Entity.class, "random").set(v, new net.minecraft.world.level.levelgen.LegacyRandomSource(randomSeed));
            v.getOffers();
            if (v instanceof Villager villager) {
                int max = Integer.parseInt(spec.split(":")[2]);
                Method update = method(Villager.class, "updateTrades", ServerLevel.class);
                for (int l = 2; l <= max; l++) {
                    villager.setVillagerData(villager.getVillagerData().withLevel(l));
                    update.invoke(villager, level);
                }
            }
            return v;
        }

        String offersNbt(ServerLevel level, MerchantOffers offers) {
            return canon(MerchantOffers.CODEC.encodeStart(level.registryAccess().createSerializationContext(NbtOps.INSTANCE), offers).getOrThrow());
        }

        String stack(ServerLevel level, ItemStack s) {
            if (s.isEmpty()) return "-";
            return canon(ItemStack.CODEC.encodeStart(level.registryAccess().createSerializationContext(NbtOps.INSTANCE), s).getOrThrow());
        }

        double[] origin(Map<String, String> kv) {
            String[] o = kv.getOrDefault("origin", "0.5,-60,0.5").split(",");
            return new double[] {Double.parseDouble(o[0]), Double.parseDouble(o[1]), Double.parseDouble(o[2])};
        }

        void run(String[] c, ServerLevel level) throws Exception {
            String kind = c[0], name = c[1];
            Map<String, String> kv = kv(c);
            long seed = Long.parseLong(kv.getOrDefault("seed", "0"));
            resetSequences(level, seed);
            double[] at = origin(kv);
            switch (kind) {
                case "gen" -> {
                    int k = 0;
                    for (String spec : kv.get("v").split(" ")) {
                        AbstractVillager v = merchant(level, spec, at, null);
                        out.println(name + " v" + (k++) + " " + offersNbt(level, v.getOffers()));
                    }
                    StringBuilder sb = new StringBuilder(name + " draws");
                    level.registryAccess().lookupOrThrow(Registries.TRADE_SET).listElements()
                        .map(h -> h.value().randomSequence())
                        .flatMap(Optional::stream)
                        .map(Identifier::toString)
                        .sorted()
                        .forEach(id -> sb.append(' ').append(this.getRandomSequence(Identifier.parse(id)).nextInt()));
                    out.println(sb);
                }
                case "price" -> {
                    Villager v = (Villager) merchant(level, kv.get("v"), at, null);
                    prepare(v, kv);
                    ServerPlayer p = player(level);
                    if (kv.containsKey("hero")) p.addEffect(new MobEffectInstance(MobEffects.HERO_OF_THE_VILLAGE, 1000, Integer.parseInt(kv.get("hero"))));
                    method(Villager.class, "updateSpecialPrices", Player.class).invoke(v, p);
                    StringBuilder sb = new StringBuilder(name + " rep " + v.getPlayerReputation(p));
                    for (MerchantOffer o : v.getOffers()) sb.append(' ').append(o.getCostA().getCount()).append('/').append(o.getSpecialPriceDiff());
                    out.println(sb);
                }
                case "restock" -> {
                    Villager v = (Villager) merchant(level, kv.get("v"), at, null);
                    prepare(v, kv);
                    var clock = level.registryAccess().lookupOrThrow(Registries.WORLD_CLOCK).getOrThrow(WorldClocks.OVERWORLD);
                    int k = 0;
                    for (String s : kv.getOrDefault("steps", "").split(",")) {
                        if (s.isEmpty()) continue;
                        String[] f = s.split(":");
                        StringBuilder sb = new StringBuilder(name + " " + (k++) + " " + s);
                        switch (f[0]) {
                            case "use" -> {
                                if (Integer.parseInt(f[1]) >= v.getOffers().size()) break;
                                MerchantOffer o = v.getOffers().get(Integer.parseInt(f[1]));
                                for (int n = Integer.parseInt(f[2]); n > 0; n--) o.increaseUses();
                            }
                            case "check" -> {
                                this.getWorldData().overworldData().setGameTime(Long.parseLong(f[1]));
                                this.clockManager().setTotalTicks(clock, Long.parseLong(f[2]) * 24000L + 1000L);
                                boolean should = v.shouldRestock(level);
                                if (should) v.restock();
                                sb.append(' ').append(should);
                            }
                            default -> throw new IllegalArgumentException(s);
                        }
                        sb.append(" lr ").append(field(Villager.class, "lastRestockGameTime").getLong(v))
                            .append(" rt ").append(field(Villager.class, "numberOfRestocksToday").getInt(v))
                            .append(" cd ").append(field(Villager.class, "lastRestockCheckDay").getLong(v));
                        for (MerchantOffer o : v.getOffers()) sb.append(' ').append(o.getUses()).append('/').append(o.getDemand());
                        out.println(sb);
                    }
                }
                case "nbt" -> {
                    AbstractVillager v = merchant(level, kv.get("v"), at, null);
                    if (v instanceof Villager villager) prepare(villager, kv);
                    TagValueOutput output = TagValueOutput.createWithContext(ProblemReporter.DISCARDING, level.registryAccess());
                    v.saveWithoutId(output);
                    CompoundTag tag = output.buildResult();
                    out.println(name + " snbt " + tag);
                    CompoundTag kept = new CompoundTag();
                    for (String key : List.of("Offers", "Xp", "LastRestock", "RestocksToday", "VillagerData")) {
                        if (tag.contains(key)) kept.put(key, tag.get(key));
                    }
                    out.println(name + " canon " + canon(kept));
                }
                case "trade" -> trade(name, kv, level, at);
                default -> throw new IllegalArgumentException(kind);
            }
        }

        /** Uses, demand, xp, gossip and restock state set on a villager before a case runs. */
        void prepare(Villager v, Map<String, String> kv) throws Exception {
            for (String key : List.of("uses", "demand")) {
                if (!kv.containsKey(key)) continue;
                for (String s : kv.get(key).split(" ")) {
                    String[] f = s.split(":");
                    // Offers past the end are skipped.
                    if (Integer.parseInt(f[0]) >= v.getOffers().size()) continue;
                    MerchantOffer o = v.getOffers().get(Integer.parseInt(f[0]));
                    if (key.equals("uses")) {
                        for (int n = Integer.parseInt(f[1]); n > 0; n--) o.increaseUses();
                    } else {
                        field(MerchantOffer.class, "demand").setInt(o, Integer.parseInt(f[1]));
                    }
                }
            }
            if (kv.containsKey("gossip")) {
                for (String s : kv.get("gossip").split(" ")) {
                    String[] f = s.split(":");
                    v.getGossips().add(PLAYER_UUID, GossipType.valueOf(f[0].toUpperCase(java.util.Locale.ROOT)), Integer.parseInt(f[1]));
                }
            }
            if (kv.containsKey("xp")) v.setVillagerXp(Integer.parseInt(kv.get("xp")));
            if (kv.containsKey("lastrestock")) field(Villager.class, "lastRestockGameTime").setLong(v, Long.parseLong(kv.get("lastrestock")));
            if (kv.containsKey("restocks")) field(Villager.class, "numberOfRestocksToday").setInt(v, Integer.parseInt(kv.get("restocks")));
        }

        void trade(String name, Map<String, String> kv, ServerLevel level, double[] at) throws Exception {
            long vseed = Long.parseLong(kv.getOrDefault("vseed", "0"));
            AbstractVillager v = merchant(level, kv.get("v"), at, vseed);
            if (v instanceof Villager villager) prepare(villager, kv);
            level.addFreshEntity(v);
            ServerPlayer p = player(level);
            if (kv.containsKey("hero")) p.addEffect(new MobEffectInstance(MobEffects.HERO_OF_THE_VILLAGE, 1000, Integer.parseInt(kv.get("hero"))));
            if (kv.containsKey("inv")) {
                for (String s : kv.get("inv").split(",")) {
                    // SLOT:ITEM*COUNT
                    String[] f = s.split(":", 2);
                    String[] ic = f[1].split("\\*");
                    p.getInventory().setItem(Integer.parseInt(f[0]), new ItemStack(
                        BuiltInRegistries.ITEM.getValue(Identifier.parse(ic[0].contains(":") ? ic[0] : "minecraft:" + ic[0])), Integer.parseInt(ic[1])));
                }
            }
            p.getInventory().setSelectedSlot(Integer.parseInt(kv.getOrDefault("hand", "8")));
            var result = v.interact(p, InteractionHand.MAIN_HAND, p.position());
            out.println(name + " open " + (p.containerMenu instanceof MerchantMenu) + " unhappy " + v.getUnhappyCounter());
            int k = 0;
            for (String s : kv.getOrDefault("steps", "").split(",")) {
                if (s.isEmpty()) continue;
                String[] f = s.split(":");
                switch (f[0]) {
                    case "select" -> {
                        if (p.containerMenu instanceof MerchantMenu menu && menu.stillValid(p)) {
                            menu.setSelectionHint(Integer.parseInt(f[1]));
                            menu.tryMoveItems(Integer.parseInt(f[1]));
                        }
                    }
                    case "click" -> p.containerMenu.clicked(Integer.parseInt(f[1]), Integer.parseInt(f[2]),
                        ContainerInput.valueOf(f[3].toUpperCase(java.util.Locale.ROOT)), p);
                    case "tick" -> {
                        // Villager.customServerAiStep: the TRADE reputation event of the last trade.
                        if (v instanceof Villager villager) {
                            Field last = field(Villager.class, "lastTradedPlayer");
                            Object who = last.get(villager);
                            if (who != null) {
                                level.onReputationEvent(ReputationEventType.TRADE, (Entity) who, villager);
                                last.set(villager, null);
                            }
                        }
                    }
                    case "close" -> p.closeContainer();
                    default -> throw new IllegalArgumentException(s);
                }
                StringBuilder sb = new StringBuilder(name + " " + (k++) + " " + s);
                var menu = p.containerMenu;
                sb.append(" menu ").append(menu instanceof MerchantMenu);
                if (menu instanceof MerchantMenu mm) {
                    for (int i = 0; i < 3; i++) sb.append(' ').append(stack(level, mm.getSlot(i).getItem()));
                    sb.append(" fxp ").append(mm.getFutureTraderXp());
                }
                sb.append(" carried ").append(stack(level, menu.getCarried()));
                sb.append(" inv");
                for (int i = 0; i < 36; i++) {
                    ItemStack st = p.getInventory().getItem(i);
                    if (!st.isEmpty()) sb.append(' ').append(i).append('=').append(stack(level, st));
                }
                sb.append(" xp ").append(v.getVillagerXp());
                if (v instanceof Villager villager) {
                    sb.append(" lvl ").append(villager.getVillagerData().level())
                        .append(" rep ").append(villager.getPlayerReputation(p))
                        .append(" regen ").append(villager.hasEffect(MobEffects.REGENERATION));
                }
                sb.append(" offers");
                for (MerchantOffer o : v.getOffers()) {
                    sb.append(' ').append(o.getUses()).append('/').append(o.getSpecialPriceDiff()).append('/').append(o.getCostA().getCount());
                }
                sb.append(" orbs");
                for (ExperienceOrb orb : level.getEntitiesOfClass(ExperienceOrb.class, new AABB(at[0] - 8, at[1] - 8, at[2] - 8, at[0] + 8, at[1] + 8, at[2] + 8))) {
                    sb.append(' ').append(orb.getValue());
                    orb.discard();
                }
                out.println(sb);
            }
            out.println(name + " next " + ((net.minecraft.util.RandomSource) field(Entity.class, "random").get(v)).nextInt());
            if (p.containerMenu != p.inventoryMenu) p.closeContainer();
            v.discard();
        }

        @Override protected SampleLogger getTickTimeLogger() { return sampleLogger; }
        @Override public boolean isTickTimeLoggingEnabled() { return false; }
        @Override protected void waitUntilNextTick() { this.runAllTasks(); }
        @Override public SystemReport fillServerSystemReport(SystemReport report) { report.setDetail("Type", "Trading oracle"); return report; }
        @Override protected void onServerExit() { super.onServerExit(); System.exit(0); }
        @Override protected void onServerCrash(net.minecraft.CrashReport report) { super.onServerCrash(report); System.exit(1); }
        @Override public boolean isHardcore() { return false; }
        @Override public LevelBasedPermissionSet operatorUserPermissions() { return LevelBasedPermissionSet.ALL; }
        @Override public PermissionSet getFunctionCompilationPermissions() { return LevelBasedPermissionSet.OWNER; }
        @Override public boolean shouldRconBroadcast() { return false; }
        @Override public boolean isDedicatedServer() { return false; }
        @Override public int getRateLimitPacketsPerSecond() { return 0; }
        @Override public int getCommandSpamThresholdSeconds() { return 0; }
        @Override public int getChatSpamThresholdSeconds() { return 0; }
        @Override public boolean useNativeTransport() { return false; }
        @Override public boolean isPublished() { return false; }
        @Override public boolean shouldInformAdmins() { return false; }
        @Override public boolean isSingleplayerOwner(NameAndId nameAndId) { return false; }
        @Override public int getMaxPlayers() { return 1; }
    }
}
