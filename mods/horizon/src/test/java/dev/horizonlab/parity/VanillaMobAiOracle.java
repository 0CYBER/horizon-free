package dev.horizonlab.parity;

import com.mojang.authlib.GameProfile;
import com.mojang.serialization.Lifecycle;
import io.netty.channel.embedded.EmbeddedChannel;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.net.Proxy;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.UUID;
import java.util.function.BooleanSupplier;
import net.minecraft.SharedConstants;
import net.minecraft.SystemReport;
import net.minecraft.commands.Commands;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.GlobalPos;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.gametest.framework.GameTestServer;
import net.minecraft.gizmos.GizmoCollector;
import net.minecraft.gizmos.Gizmos;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.game.ClientboundSoundEntityPacket;
import net.minecraft.network.protocol.game.ClientboundSoundPacket;
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
import net.minecraft.util.Util;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.util.debugchart.LocalSampleLogger;
import net.minecraft.util.debugchart.SampleLogger;
import net.minecraft.world.clock.WorldClocks;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.entity.Mob;
import net.minecraft.world.entity.AgeableMob;
import net.minecraft.world.entity.ai.Brain;
import net.minecraft.world.entity.ai.behavior.BehaviorControl;
import net.minecraft.world.entity.ai.behavior.BlockPosTracker;
import net.minecraft.world.entity.ai.behavior.GateBehavior;
import net.minecraft.world.entity.ai.behavior.ShufflingList;
import net.minecraft.world.entity.ai.behavior.EntityTracker;
import net.minecraft.world.entity.ai.control.LookControl;
import net.minecraft.world.entity.ai.control.MoveControl;
import net.minecraft.world.entity.ai.goal.GoalSelector;
import net.minecraft.world.entity.ai.goal.WrappedGoal;
import net.minecraft.world.entity.ai.memory.MemoryModuleType;
import net.minecraft.world.entity.ai.memory.WalkTarget;
import net.minecraft.world.entity.ai.navigation.PathNavigation;
import net.minecraft.world.entity.ai.sensing.Sensor;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.level.DataPackConfig;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.dimension.LevelStem;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.WorldDimensions;
import net.minecraft.world.level.levelgen.WorldGenSettings;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.storage.LevelDataAndDimensions;
import net.minecraft.world.level.storage.LevelStorageSource;
import net.minecraft.world.level.storage.PrimaryLevelData;
import net.minecraft.world.phys.Vec3;

/**
 * Mob AI oracle: a real (headless, flat, structure-free) vanilla server level. Each scenario
 * builds blocks, places a target player and mobs with seeded random sources, then ticks the
 * mobs exactly as ServerLevel.tick does (despawn check, then tickNonPassenger) while the rest of
 * the level stands still (no block ticks, weather, spawning or random ticks), printing the
 * AI-visible state of every mob after each tick: position, motion, rotations, running goals,
 * targets, navigation, move/look/jump controls, brain activities and memories, and sounds.
 */
public final class VanillaMobAiOracle {
    static final List<Scenario> SCENARIOS = new ArrayList<>();
    static PrintWriter out;
    static final List<String> SOUNDS = new ArrayList<>();

    /** Draws of a traced mob random: the caller frames of each draw (AI_TRACE=scenario:from-to). */
    static final class TracingRandom extends net.minecraft.world.level.levelgen.LegacyRandomSource {
        static int tick;
        static boolean on;
        final List<String> log;
        final int who;
        TracingRandom(long seed, List<String> log, int who) { super(seed); this.log = log; this.who = who; }
        @Override
        public int next(int bits) {
            int v = super.next(bits);
            if (on) {
                StringBuilder sb = new StringBuilder(tick + " trace " + who + " " + bits + "=" + Integer.toHexString(v) + ":");
                int n = 0;
                for (StackTraceElement e : new Throwable().getStackTrace()) {
                    String c = e.getClassName();
                    if (c.contains("RandomSource") || c.contains("TracingRandom") || c.contains("Mth")) continue;
                    sb.append(' ').append(c.substring(c.lastIndexOf('.') + 1)).append('.').append(e.getMethodName()).append(':').append(e.getLineNumber());
                    if (++n == 7) break;
                }
                log.add(sb.toString());
            }
            return v;
        }
    }
    static final List<String> TRACE = new ArrayList<>();

    static final class MobSpec {
        String type;
        double x, y, z;
        float yaw;
        long seed;
        boolean baby;
        List<String> flags = new ArrayList<>();
    }

    static final class Scenario {
        String name;
        int ticks;
        long time = 6000;
        boolean rain, thunder;
        int originX, originZ;
        List<String[]> blocks = new ArrayList<>();
        double[] player;
        boolean creative;
        List<MobSpec> mobs = new ArrayList<>();
        long levelSeed = 0;
    }

    static void parse(Path path) throws Exception {
        Scenario s = null;
        for (String raw : Files.readAllLines(path)) {
            String line = raw.strip();
            if (line.isEmpty() || line.startsWith("#")) continue;
            String[] f = line.split("\\s+");
            switch (f[0]) {
                case "scenario" -> {
                    s = new Scenario();
                    s.name = f[1];
                    for (int i = 2; i < f.length; i++) {
                        String[] kv = f[i].split("=", 2);
                        switch (kv[0]) {
                            case "ticks" -> s.ticks = Integer.parseInt(kv[1]);
                            case "time" -> s.time = Long.parseLong(kv[1]);
                            case "rain" -> s.rain = true;
                            case "thunder" -> s.thunder = true;
                            case "levelseed" -> s.levelSeed = Long.parseLong(kv[1]);
                            default -> throw new IllegalArgumentException(line);
                        }
                    }
                }
                case "origin" -> { s.originX = Integer.parseInt(f[1]); s.originZ = Integer.parseInt(f[2]); }
                case "fill", "block" -> s.blocks.add(f);
                case "player" -> {
                    s.player = new double[] {Double.parseDouble(f[1]), Double.parseDouble(f[2]), Double.parseDouble(f[3])};
                    s.creative = f.length > 4 && f[4].equals("creative");
                }
                case "mob" -> {
                    MobSpec m = new MobSpec();
                    m.type = f[1];
                    m.x = Double.parseDouble(f[2]);
                    m.y = Double.parseDouble(f[3]);
                    m.z = Double.parseDouble(f[4]);
                    m.yaw = Float.parseFloat(f[5]);
                    m.seed = Long.parseLong(f[6]);
                    for (int i = 7; i < f.length; i++) {
                        if (f[i].equals("baby")) m.baby = true; else m.flags.add(f[i]);
                    }
                    s.mobs.add(m);
                }
                case "end" -> { SCENARIOS.add(s); s = null; }
                default -> throw new IllegalArgumentException(line);
            }
        }
    }

    public static void main(String[] args) throws Exception {
        parse(Path.of(args[0]));
        out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])));
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Util.startTimerHackThread();
        Path universe = Files.createTempDirectory("horizon-mob-ai");
        LevelStorageSource.LevelStorageAccess access = LevelStorageSource.createDefault(universe).createAccess("aiworld");
        PackRepository packs = ServerPacksSource.createPackRepository(access);
        MinecraftServer.spin(thread -> Server.create(thread, access, packs));
    }

    static String traceSpec() { String v = System.getenv("AI_TRACE"); return v == null ? "" : v; }
    // Every mob random is traced unless AI_TRACE=0.
    static boolean tracing(Scenario s) { return !traceSpec().equals("0"); }
    static int traceFrom() { return 0; }
    static int traceTo() { return Integer.MAX_VALUE; }

    static String d(double v) { return Long.toHexString(Double.doubleToRawLongBits(v)); }
    static String f(float v) { return Integer.toHexString(Float.floatToRawIntBits(v)); }

    static Field field(Class<?> c, String name) throws Exception {
        for (Class<?> k = c; k != null; k = k.getSuperclass()) {
            try {
                Field fl = k.getDeclaredField(name);
                fl.setAccessible(true);
                return fl;
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

    static Object get(Object o, String name) throws Exception { return field(o.getClass(), name).get(o); }

    static final class Server extends MinecraftServer {
        static final FeatureFlagSet FEATURES = FeatureFlags.REGISTRY
            .allFlags()
            .subtract(FeatureFlagSet.of(FeatureFlags.REDSTONE_EXPERIMENTS, FeatureFlags.MINECART_IMPROVEMENTS));
        final LocalSampleLogger sampleLogger = new LocalSampleLogger(4);
        int scenario = 0;
        int phase = 0;
        int wait = 0;
        ServerPlayer player;
        int failures = 0;

        static Server create(Thread thread, LevelStorageSource.LevelStorageAccess access, PackRepository packs) {
            try {
                packs.reload();
                ArrayList<String> enabled = new ArrayList<>(packs.getAvailableIds());
                enabled.remove("vanilla");
                enabled.addFirst("vanilla");
                WorldDataConfiguration config = new WorldDataConfiguration(new DataPackConfig(enabled, List.of()), FEATURES);
                LevelSettings settings = new LevelSettings("AI Level", GameType.SURVIVAL, LevelSettings.DifficultySettings.DEFAULT, true, config);
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
                    capture(packet);
                }
            });
            Gizmos.withCollector(GizmoCollector.NOOP);
            this.loadLevel();
            return true;
        }

        static void capture(Packet<?> packet) {
            if (packet instanceof ClientboundSoundPacket p) {
                SOUNDS.add(p.getSound().value().location() + " " + f(p.getVolume()) + " " + f(p.getPitch()));
            } else if (packet instanceof ClientboundSoundEntityPacket p) {
                SOUNDS.add(p.getSound().value().location() + " " + f(p.getVolume()) + " " + f(p.getPitch()));
            }
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

        ServerLevel level() { return this.overworld(); }

        void step(BooleanSupplier haveTime) throws Exception {
            if (scenario >= SCENARIOS.size()) {
                out.close();
                // Nothing to save: skip the shutdown.
                Runtime.getRuntime().halt(0);
                return;
            }
            Scenario s = SCENARIOS.get(scenario);
            ServerLevel level = level();
            int cx = s.originX >> 4, cz = s.originZ >> 4;
            switch (phase) {
                case 0 -> {
                    for (int dx = -4; dx <= 4; dx++)
                        for (int dz = -4; dz <= 4; dz++)
                            level.setChunkForced(cx + dx, cz + dz, true);
                    phase = 1;
                    super.tickServer(haveTime);
                }
                case 1 -> {
                    super.tickServer(haveTime);
                    boolean ready = true;
                    for (int dx = -4; dx <= 4 && ready; dx++)
                        for (int dz = -4; dz <= 4 && ready; dz++)
                            ready = level.isPositionEntityTicking(new BlockPos((cx + dx) * 16, -60, (cz + dz) * 16));
                    if (ready) {
                        build(s, level);
                        phase = 2;
                        wait = 0;
                    }
                }
                case 2 -> {
                    super.tickServer(haveTime);
                    if (!level.getChunkSource().getLightEngine().hasLightWork() && ++wait > 5) {
                        run(s, level);
                        for (int dx = -4; dx <= 4; dx++)
                            for (int dz = -4; dz <= 4; dz++)
                                level.setChunkForced(cx + dx, cz + dz, false);
                        scenario++;
                        phase = 0;
                    }
                }
                default -> throw new IllegalStateException();
            }
        }

        void build(Scenario s, ServerLevel level) throws Exception {
            var blocks = level.holderLookup(Registries.BLOCK);
            for (String[] b : s.blocks) {
                if (b[0].equals("block")) {
                    BlockState state = BlockStateParser.parseForBlock(blocks, b[4], false).blockState();
                    level.setBlock(new BlockPos(Integer.parseInt(b[1]), Integer.parseInt(b[2]), Integer.parseInt(b[3])), state, 2 | 16);
                } else {
                    BlockState state = BlockStateParser.parseForBlock(blocks, b[7], false).blockState();
                    int x0 = Integer.parseInt(b[1]), y0 = Integer.parseInt(b[2]), z0 = Integer.parseInt(b[3]);
                    int x1 = Integer.parseInt(b[4]), y1 = Integer.parseInt(b[5]), z1 = Integer.parseInt(b[6]);
                    for (int x = Math.min(x0, x1); x <= Math.max(x0, x1); x++)
                        for (int y = Math.min(y0, y1); y <= Math.max(y0, y1); y++)
                            for (int z = Math.min(z0, z1); z <= Math.max(z0, z1); z++)
                                level.setBlock(new BlockPos(x, y, z), state, 2 | 16);
                }
            }
        }

        void run(Scenario s, ServerLevel level) throws Exception {
            out.println("== " + s.name);
            var clock = level.registryAccess().lookupOrThrow(Registries.WORLD_CLOCK).getOrThrow(WorldClocks.OVERWORLD);
            this.clockManager().setTotalTicks(clock, s.time);
            // Every scenario starts at game time 1000.
            this.getWorldData().overworldData().setGameTime(1000L);
            // isRaining/isThundering read the rain and thunder levels only.
            level.setRainLevel(s.rain ? 1.0F : 0.0F);
            level.setThunderLevel(s.thunder ? 1.0F : 0.0F);
            if (s.player != null) {
                if (player == null) {
                    CommonListenerCookie cookie = CommonListenerCookie.createInitial(new GameProfile(new UUID(0x1234L, 0x5678L), "horizon-ai"), false);
                    player = new ServerPlayer(this, level, cookie.gameProfile(), cookie.clientInformation());
                    Connection connection = new Connection(PacketFlow.SERVERBOUND);
                    new EmbeddedChannel(connection);
                    this.getPlayerList().placeNewPlayer(connection, player, cookie);
                }
                player.setGameMode(s.creative ? GameType.CREATIVE : GameType.SURVIVAL);
                player.snapTo(s.player[0], s.player[1], s.player[2], 0.0F, 0.0F);
                player.setDeltaMovement(Vec3.ZERO);
            } else if (player != null) {
                player.snapTo(s.originX + 5000, -60, s.originZ, 0.0F, 0.0F);
            }
            List<Mob> mobs = new ArrayList<>();
            List<Brain<?>> brains = new ArrayList<>();
            List<Long> seeds = new ArrayList<>();
            int id = 100000;
            for (MobSpec spec : s.mobs) {
                EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getValue(net.minecraft.resources.Identifier.parse(spec.type));
                Mob mob = (Mob) type.create(level, EntitySpawnReason.COMMAND);
                if (tracing(s)) field(Entity.class, "random").set(mob, new TracingRandom(0L, TRACE, mobs.size()));
                mob.setId(id++);
                mob.snapTo(spec.x, spec.y, spec.z, spec.yaw, 0.0F);
                mob.setYHeadRot(spec.yaw);
                mob.setYBodyRot(spec.yaw);
                if (spec.baby) {
                    if (mob instanceof AgeableMob a) a.setBaby(true);
                    else mob.setBaby(true);
                }
                for (String flag : spec.flags) {
                    if (flag.equals("persistent")) mob.setPersistenceRequired();
                    else if (flag.equals("noai")) mob.setNoAi(true);
                    else throw new IllegalArgumentException(flag);
                }
                level.addFreshEntity(mob);
                mob.getRandom().setSeed(spec.seed);
                Brain<?> brain = mob.getBrain();
                @SuppressWarnings("unchecked")
                Map<Object, Sensor<?>> sensors = (Map<Object, Sensor<?>>) get(brain, "sensors");
                for (Sensor<?> sensor : sensors.values()) sensor.randomlyDelayStart(mob.getRandom());
                seedGates(brain, spec.seed);
                mobs.add(mob);
                brains.add(brain);
                seeds.add(spec.seed);
            }
            level.getRandom().setSeed(s.levelSeed);
            SOUNDS.clear();
            Method tickTime = method(ServerLevel.class, "tickTime");
            Object manager = get(level, "entityManager");
            Method managerTick = method(manager.getClass(), "tick");
            for (int i = 0; i < mobs.size(); i++) print(0, i, mobs.get(i));
            for (int t = 1; t <= s.ticks; t++) {
                level.environmentAttributes().invalidateTickCache();
                level.updateSkyBrightness();
                tickTime.invoke(level);
                if (player != null && s.player != null) {
                    player.setHealth(player.getMaxHealth());
                    player.setInvulnerableTime(0);
                    player.hurtTime = 0;
                    player.snapTo(s.player[0], s.player[1], s.player[2], 0.0F, 0.0F);
                    player.setDeltaMovement(Vec3.ZERO);
                }
                TracingRandom.tick = t;
                TracingRandom.on = tracing(s) && t >= traceFrom() && t <= traceTo();
                SOUNDS.clear();
                for (int i = 0; i < mobs.size(); i++) {
                    // Villager.refreshBrain builds a new brain with new gates: seed those too.
                    if (mobs.get(i).getBrain() != brains.get(i)) {
                        brains.set(i, mobs.get(i).getBrain());
                        seedGates(brains.get(i), seeds.get(i));
                    }
                }
                for (Mob mob : mobs) {
                    if (mob.isRemoved()) continue;
                    mob.checkDespawn();
                    if (mob.isRemoved()) continue;
                    level.tickNonPassenger(mob);
                }
                managerTick.invoke(manager);
                for (int i = 0; i < mobs.size(); i++) print(t, i, mobs.get(i));
                for (String sound : SOUNDS) out.println(t + " sound " + sound);
                for (String line : TRACE) out.println(line);
                TRACE.clear();
            }
            for (Mob mob : mobs) mob.discard();
            managerTick.invoke(manager);
            out.flush();
        }

        /**
         * Every GateBehavior (RunOne and the gates of TradeWithVillager and VillagerMakeLove) shuffles
         * its behaviours with its own ShufflingList random, which RandomSource.create() seeds from the
         * clock: give each a LegacyRandomSource seeded with the mob's seed so runs are reproducible.
         */
        static void seedGates(Brain<?> brain, long seed) throws Exception {
            @SuppressWarnings("unchecked")
            Map<Integer, Map<?, Set<BehaviorControl<?>>>> byPriority = (Map<Integer, Map<?, Set<BehaviorControl<?>>>>) get(brain, "availableBehaviorsByPriority");
            for (Map<?, Set<BehaviorControl<?>>> byActivity : byPriority.values())
                for (Set<BehaviorControl<?>> behaviors : byActivity.values())
                    for (BehaviorControl<?> behavior : behaviors) seedGate(behavior, seed);
        }

        static void seedGate(Object behavior, long seed) throws Exception {
            if (!(behavior instanceof GateBehavior<?>)) return;
            ShufflingList<?> list = (ShufflingList<?>) field(GateBehavior.class, "behaviors").get(behavior);
            field(ShufflingList.class, "random").set(list, new net.minecraft.world.level.levelgen.LegacyRandomSource(seed));
            for (Object child : list) seedGate(child, seed);
        }

        static String who(Entity e) {
            if (e == null) return "none";
            if (e instanceof ServerPlayer) return "player";
            return "e" + e.getId();
        }

        static String goals(GoalSelector selector) {
            StringBuilder sb = new StringBuilder();
            for (WrappedGoal g : selector.getAvailableGoals()) {
                if (!g.isRunning()) continue;
                if (sb.length() > 0) sb.append('|');
                Class<?> c = g.getGoal().getClass();
                String name = c.getSimpleName();
                if (name.isEmpty()) name = c.getName().substring(c.getName().lastIndexOf('.') + 1);
                sb.append(name);
            }
            return sb.length() == 0 ? "-" : sb.toString();
        }

        static String pos(BlockPos p) { return p.getX() + "," + p.getY() + "," + p.getZ(); }

        static String value(Object v) {
            if (v == null) return "null";
            if (v instanceof Entity e) return who(e);
            if (v instanceof BlockPos p) return pos(p);
            if (v instanceof GlobalPos g) return pos(g.pos());
            if (v instanceof WalkTarget w) return value(w.getTarget()) + "/" + f(w.getSpeedModifier()) + "/" + w.getCloseEnoughDist();
            if (v instanceof EntityTracker e) return who(e.getEntity());
            if (v instanceof BlockPosTracker b) return pos(b.currentBlockPosition());
            if (v instanceof Boolean || v instanceof Integer || v instanceof Long) return v.toString();
            if (v instanceof net.minecraft.util.Unit) return "u";
            if (v instanceof Collection<?> c) return "n" + c.size();
            if (v instanceof net.minecraft.world.level.pathfinder.Path p) return "path" + pos(p.getTarget());
            return v.getClass().getSimpleName();
        }

        void print(int t, int i, Mob m) throws Exception {
            StringBuilder sb = new StringBuilder();
            sb.append(t).append(' ').append(i);
            if (m.isRemoved()) {
                out.println(sb.append(" removed ").append(m.getRemovalReason()));
                return;
            }
            sb.append("\tp ").append(d(m.getX())).append(',').append(d(m.getY())).append(',').append(d(m.getZ()));
            Vec3 v = m.getDeltaMovement();
            sb.append("\tv ").append(d(v.x)).append(',').append(d(v.y)).append(',').append(d(v.z));
            sb.append("\tr ").append(f(m.getYRot())).append(',').append(f(m.getXRot())).append(',').append(f(m.getYHeadRot())).append(',').append(f(m.yBodyRot));
            sb.append("\tg ").append(m.onGround() ? 1 : 0).append(m.horizontalCollision ? 1 : 0);
            sb.append("\thp ").append(f(m.getHealth()));
            sb.append("\tgoals ").append(goals((GoalSelector) get(m, "goalSelector")));
            sb.append("\ttargets ").append(goals((GoalSelector) get(m, "targetSelector")));
            sb.append("\ttarget ").append(who(m.getTarget()));
            PathNavigation nav = m.getNavigation();
            var path = nav.getPath();
            sb.append("\tnav ");
            if (path == null) sb.append("none");
            else sb.append(pos(path.getTarget())).append(' ').append(path.getNextNodeIndex()).append('/').append(path.getNodeCount())
                .append(path.isDone() ? " done" : "");
            MoveControl mc = m.getMoveControl();
            sb.append("\tmove ").append(get(mc, "operation")).append(' ').append(d((double) get(mc, "wantedX"))).append(',')
                .append(d((double) get(mc, "wantedY"))).append(',').append(d((double) get(mc, "wantedZ"))).append(' ')
                .append(d((double) get(mc, "speedModifier")));
            LookControl lc = m.getLookControl();
            sb.append("\tlook ").append(get(lc, "lookAtCooldown")).append(' ').append(d((double) get(lc, "wantedX"))).append(',')
                .append(d((double) get(lc, "wantedY"))).append(',').append(d((double) get(lc, "wantedZ")));
            sb.append("\tin ").append(f(m.xxa)).append(',').append(f(m.zza)).append(',').append(f(m.getSpeed())).append(',')
                .append(field(LivingEntity.class, "jumping").getBoolean(m) ? 1 : 0);
            sb.append("\trng ").append(Long.toHexString(((java.util.concurrent.atomic.AtomicLong) field(net.minecraft.world.level.levelgen.LegacyRandomSource.class, "seed").get(m.getRandom())).get()));
            Brain<?> brain = m.getBrain();
            if (!brain.getActiveActivities().isEmpty() && !((Map<?, ?>) get(brain, "sensors")).isEmpty()) {
                List<String> acts = new ArrayList<>();
                for (var a : brain.getActiveActivities()) acts.add(a.getName());
                acts.sort(null);
                sb.append("\tact ").append(String.join("|", acts));
                List<String> mem = new ArrayList<>();
                brain.forEach(new Brain.Visitor() {
                    @Override public <U> void acceptEmpty(MemoryModuleType<U> type) {}
                    @Override public <U> void accept(MemoryModuleType<U> type, U value) { mem.add(name(type) + "=" + value(value)); }
                    @Override public <U> void accept(MemoryModuleType<U> type, U value, long ttl) { mem.add(name(type) + "=" + value(value) + "@" + ttl); }
                });
                mem.sort(null);
                sb.append("\tmem ").append(String.join(" ", mem));
            }
            out.println(sb);
        }

        static String name(MemoryModuleType<?> type) {
            return BuiltInRegistries.MEMORY_MODULE_TYPE.getKey(type).getPath();
        }

        @Override protected SampleLogger getTickTimeLogger() { return sampleLogger; }
        @Override public boolean isTickTimeLoggingEnabled() { return false; }
        @Override protected void waitUntilNextTick() { this.runAllTasks(); }
        @Override public SystemReport fillServerSystemReport(SystemReport report) { report.setDetail("Type", "Mob AI oracle"); return report; }
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
