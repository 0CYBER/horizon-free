package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.function.Predicate;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.nbt.*;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientboundSoundPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.PlayerAdvancements;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.level.ServerBossEvent;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.ServerGamePacketListenerImpl;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.stats.Stat;
import net.minecraft.tags.TagLoader;
import net.minecraft.util.Mth;
import net.minecraft.util.RandomSource;
import net.minecraft.world.BossEvent;
import net.minecraft.world.Difficulty;
import net.minecraft.world.DifficultyInstance;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.monster.PatrollingMonster;
import net.minecraft.world.entity.raid.Raid;
import net.minecraft.world.entity.raid.Raider;
import net.minecraft.world.entity.raid.Raids;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeManager;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.dimension.DimensionType;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.levelgen.PatrolSpawner;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.storage.LevelData;
import net.minecraft.world.phys.Vec3;

/**
 * Original oracle: pillager patrols and raids as vanilla runs them, on a
 * stand-in ServerLevel built from boxes of blocks and light, one biome,
 * village sections (the POI distance tracker's answer), unloaded and
 * non-ticking chunks, and at most one player. Three modes:
 *
 * - patrol: PatrolSpawner.tick for some ticks with the level random seeded;
 *   finalizeSpawn reads a fixed "neutral" random (nextInt 0, nextFloat 0.75)
 *   so its own draws stay out of the spawner's sequence;
 * - raid: Raids.tick over scripted ticks, with the raid's random seeded and
 *   scripted raider events (hurt, kill, move, idle, join, load, leave the
 *   level), omens through Raids.createOrExtendRaid, and world changes;
 * - codec: Raids.CODEC decoding an SNBT tag with resultOrPartial (else a fresh
 *   Raids), then encoding it.
 *
 * Each case prints one line of events in a format the Rust fixture
 * (raid_fixture) reproduces.
 */
public final class VanillaRaidOracle {
    static Object unsafe;
    static Method allocate, putObject, objectFieldOffset;

    static Object alloc(Class<?> c) throws Exception { return allocate.invoke(unsafe, c); }

    static void put(Object target, Class<?> owner, String field, Object value) throws Exception {
        Field f = owner.getDeclaredField(field);
        putObject.invoke(unsafe, target, (long) objectFieldOffset.invoke(unsafe, f), value);
    }

    static Object get(Object target, Class<?> owner, String field) throws Exception {
        Field f = owner.getDeclaredField(field);
        f.setAccessible(true);
        return f.get(target);
    }

    record Box(int x0, int y0, int z0, int x1, int y1, int z1) {
        boolean has(int x, int y, int z) { return x >= x0 && x <= x1 && y >= y0 && y <= y1 && z >= z0 && z <= z1; }
        static Box parse(String s) {
            String[] p = s.split(",");
            return new Box(Integer.parseInt(p[0]), Integer.parseInt(p[1]), Integer.parseInt(p[2]),
                Integer.parseInt(p[3]), Integer.parseInt(p[4]), Integer.parseInt(p[5]));
        }
    }

    /** finalizeSpawn's random: the same answer every time. */
    static final class Neutral implements RandomSource {
        @Override public RandomSource fork() { return this; }
        @Override public net.minecraft.world.level.levelgen.PositionalRandomFactory forkPositional() { throw new UnsupportedOperationException(); }
        @Override public void setSeed(long seed) {}
        @Override public int nextInt() { return 0; }
        @Override public int nextInt(int bound) { return 0; }
        @Override public long nextLong() { return 0L; }
        @Override public boolean nextBoolean() { return false; }
        @Override public float nextFloat() { return 0.75F; }
        @Override public double nextDouble() { return 0.75; }
        @Override public double nextGaussian() { return 0.0; }
    }

    static final class Lvl extends ServerLevel {
        Lvl() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        RegistryAccess registries;
        net.minecraft.server.MinecraftServer fake;
        @Override public net.minecraft.server.MinecraftServer getServer() { return fake; }
        List<Map.Entry<Box, BlockState>> blocks = new ArrayList<>();
        List<Map.Entry<Box, int[]>> lights = new ArrayList<>();
        Holder<Biome> biome;
        int sky, block, darken;
        Difficulty difficulty;
        DimensionType dimensionType;
        ResourceKey<Level> dimensionKey;
        Set<Long> unloaded = new HashSet<>();
        Set<Long> noTicking = new HashSet<>();
        List<int[]> villages = new ArrayList<>();
        List<BlockPos> pois = new ArrayList<>();
        List<Entity> entities = new ArrayList<>();
        List<ServerPlayer> playerList = new ArrayList<>();
        net.minecraft.world.attribute.EnvironmentAttributeSystem attributes;
        BiomeManager biomeManager;
        GameRules rules;
        RandomSource neutral = new Neutral();
        RandomSource main;
        boolean finalizing, grouped;
        long clock, inhabited;
        float moon;
        List<String> log;
        @Override public RegistryAccess registryAccess() { return registries; }
        @Override public Difficulty getDifficulty() { return difficulty; }
        @Override public long getGameTime() { return 0L; }
        int next = 1;
        @Override public int getNextEntityId() { return next++; }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags(); }
        @Override public BlockState getBlockState(BlockPos pos) {
            for (int i = blocks.size() - 1; i >= 0; i--)
                if (blocks.get(i).getKey().has(pos.getX(), pos.getY(), pos.getZ())) return blocks.get(i).getValue();
            return Blocks.AIR.defaultBlockState();
        }
        @Override public FluidState getFluidState(BlockPos pos) { return getBlockState(pos).getFluidState(); }
        @Override public BlockGetter getChunkForCollisions(int chunkX, int chunkZ) { return this; }
        int[] light(BlockPos pos) {
            for (int i = lights.size() - 1; i >= 0; i--)
                if (lights.get(i).getKey().has(pos.getX(), pos.getY(), pos.getZ())) return lights.get(i).getValue();
            return new int[] {sky, block};
        }
        @Override public int getBrightness(LightLayer layer, BlockPos pos) { int[] l = light(pos); return layer == LightLayer.SKY ? l[0] : l[1]; }
        @Override public int getRawBrightness(BlockPos pos, int darkening) { int[] l = light(pos); return Math.max(l[1], l[0] - darkening); }
        @Override public int getSkyDarken() { return darken; }
        @Override public boolean isBrightOutside() { return !dimensionType.hasFixedTime() && darken < 4; }
        @Override public boolean hasChunk(int x, int z) { return !unloaded.contains(ChunkPos.pack(x, z)); }
        @Override public boolean isPositionEntityTicking(BlockPos pos) {
            long key = ChunkPos.pack(pos.getX() >> 4, pos.getZ() >> 4);
            return !unloaded.contains(key) && !noTicking.contains(key);
        }
        @Override public int getHeight(Heightmap.Types type, int x, int z) {
            if (!hasChunk(x >> 4, z >> 4)) return getMinY();
            for (int y = getMaxY(); y >= getMinY(); y--)
                if (type.isOpaque().test(getBlockState(new BlockPos(x, y, z)))) return y + 1;
            return getMinY();
        }
        @Override public int sectionsToVillage(SectionPos pos) {
            int best = 7;
            for (int[] v : villages)
                best = Math.min(best, Math.max(Math.abs(v[0] - pos.x()), Math.max(Math.abs(v[1] - pos.y()), Math.abs(v[2] - pos.z()))));
            return best;
        }
        @Override public DimensionType dimensionType() { return dimensionType; }
        @Override public ResourceKey<Level> dimension() { return dimensionKey; }
        @Override public Holder<Biome> getBiome(BlockPos pos) { return biome; }
        @Override public BiomeManager getBiomeManager() { return biomeManager; }
        @Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes() { return attributes; }
        @Override public GameRules getGameRules() { return rules; }
        net.minecraft.world.level.border.WorldBorder border = new net.minecraft.world.level.border.WorldBorder();
        @Override public net.minecraft.world.level.border.WorldBorder getWorldBorder() { return border; }
        net.minecraft.world.entity.ai.village.poi.PoiManager poi;
        @Override public net.minecraft.world.entity.ai.village.poi.PoiManager getPoiManager() { return poi; }
        @Override public RandomSource getRandom() { return finalizing ? neutral : main; }
        @Override public DifficultyInstance getCurrentDifficultyAt(BlockPos pos) {
            // The patrol spawner asks once for its group size; later asks come from spawnPatrolMember, right before finalizeSpawn.
            if (grouped) finalizing = true;
            grouped = true;
            boolean loaded = hasChunk(pos.getX() >> 4, pos.getZ() >> 4);
            return new DifficultyInstance(difficulty, clock, loaded ? inhabited : 0L, loaded ? moon : 0.0F);
        }
        @SuppressWarnings({"unchecked", "rawtypes"})
        @Override public List players() { return playerList; }
        @Override public List<ServerPlayer> getPlayers(Predicate<? super ServerPlayer> filter) {
            List<ServerPlayer> out = new ArrayList<>();
            for (ServerPlayer p : playerList) if (filter.test(p)) out.add(p);
            return out;
        }
        @Override public Entity getEntity(UUID uuid) {
            for (ServerPlayer p : playerList) if (p.getUUID().equals(uuid)) return p;
            for (Entity e : entities) if (e.getUUID().equals(uuid)) return e;
            return null;
        }
        @Override public boolean addFreshEntity(Entity entity) {
            entities.add(entity);
            finalizing = false;
            return true;
        }
        @Override public void addFreshEntityWithPassengers(Entity entity) {
            for (Entity e : entity.getSelfAndPassengers().toList()) {
                addFreshEntity(e);
                spawned(e);
            }
        }
        @Override public void gameEvent(Entity source, Holder<net.minecraft.world.level.gameevent.GameEvent> event, Vec3 pos) {}
        @Override public void gameEvent(Holder<net.minecraft.world.level.gameevent.GameEvent> event, Vec3 pos, net.minecraft.world.level.gameevent.GameEvent.Context context) {}
        @Override public void broadcastEntityEvent(Entity entity, byte event) {}

        void spawned(Entity e) {
            String type = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString();
            StringBuilder s = new StringBuilder("spawn " + type + " " + Mth.floor(e.getX()) + " " + Mth.floor(e.getY()) + " " + Mth.floor(e.getZ()));
            if (e.getVehicle() != null) s.append(" rider");
            if (e instanceof PatrollingMonster m) {
                boolean banner = !m.getItemBySlot(EquipmentSlot.HEAD).isEmpty();
                s.append(m.isPatrolLeader() ? " leader" : "").append(banner ? " banner" : "");
                try { if ((boolean) isPatrolling.invoke(m)) s.append(" patrolling"); } catch (Exception ex) { throw new RuntimeException(ex); }
                BlockPos t = m.getPatrolTarget();
                if (t != null) {
                    boolean origin = t.getY() == 0 && t.getX() >= -500 && t.getX() < 500 && t.getZ() >= -500 && t.getZ() < 500;
                    s.append(origin ? " target" : " target-elsewhere");
                }
            }
            if (e instanceof Raider r) s.append(" wave=").append(r.getWave()).append(r.canJoinRaid() ? " join" : "");
            log.add(s.toString());
        }
    }

    static Method isPatrolling;

    static final class Adv extends PlayerAdvancements {
        Adv() { super(null, null, null, null, null); }
        @Override public <T extends net.minecraft.advancements.CriterionTriggerInstance> Map<PlayerAdvancements.TriggerInstanceKey, T> getTriggerMapForType(
            net.minecraft.advancements.triggers.CriterionTrigger<T> type) { return null; }
    }

    static final class Conn extends ServerGamePacketListenerImpl {
        Conn() { super(null, null, null, null); }
        List<String> log;
        @Override public void send(Packet<?> packet) {
            if (packet instanceof ClientboundSoundPacket p)
                log.add("horn " + p.getSeed() + " " + bits(p.getX()) + " " + bits(p.getY()) + " " + bits(p.getZ()));
        }
    }

    static final class OraclePlayer extends ServerPlayer {
        OraclePlayer() { super(null, null, null, null); }
        GameType mode;
        boolean alive;
        int omen = -1;
        List<String> log;
        PlayerAdvancements adv;
        @Override public boolean isSpectator() { return mode == GameType.SPECTATOR; }
        @Override public boolean isCreative() { return mode == GameType.CREATIVE; }
        @Override public GameType gameMode() { return mode; }
        @Override public boolean isAlive() { return alive; }
        @Override public MobEffectInstance getEffect(Holder<MobEffect> effect) {
            return effect == MobEffects.RAID_OMEN && omen >= 0 ? new MobEffectInstance(MobEffects.RAID_OMEN, 600, omen) : null;
        }
        @Override public boolean addEffect(MobEffectInstance effect, Entity source) {
            log.add("effect " + BuiltInRegistries.MOB_EFFECT.getKey(effect.getEffect().value()) + " " + effect.getDuration() + " " + effect.getAmplifier()
                + " " + effect.isAmbient() + " " + effect.isVisible() + " " + effect.showIcon());
            return true;
        }
        @Override public void awardStat(Stat<?> stat, int count) { log.add("stat " + stat.getValue()); }
        @Override public PlayerAdvancements getAdvancements() { return adv; }
    }

    static String bits(double v) { return Long.toHexString(Double.doubleToRawLongBits(v)); }
    static String bits(float v) { return Integer.toHexString(Float.floatToRawIntBits(v)); }

    static Field raidMap, raidRandom, raidEvent, raidsField, celebrationTicks, waveSpawnPos, noActionTime, removalReason, groupToLeader;

    static String raidState(Raids raids, OraclePlayer player) throws Exception {
        @SuppressWarnings("unchecked")
        var map = (it.unimi.dsi.fastutil.ints.Int2ObjectMap<Raid>) raidMap.get(raids);
        StringBuilder s = new StringBuilder();
        for (var entry : map.int2ObjectEntrySet()) {
            Raid r = entry.getValue();
            ServerBossEvent bar = (ServerBossEvent) raidEvent.get(r);
            String name = bar.getName().getString();
            @SuppressWarnings("unchecked")
            Optional<BlockPos> wave = (Optional<BlockPos>) waveSpawnPos.get(r);
            @SuppressWarnings("unchecked")
            Map<Integer, Raider> leaders = (Map<Integer, Raider>) groupToLeader.get(r);
            s.append(" ; r").append(entry.getIntKey())
                .append(" ").append(r.isStopped() ? "stopped" : r.isVictory() ? "victory" : r.isLoss() ? "loss" : "ongoing")
                .append(" act=").append(r.isActive() ? 1 : 0)
                .append(" st=").append(r.isStarted() ? 1 : 0)
                .append(" om=").append(r.getRaidOmenLevel())
                .append(" gs=").append(r.getGroupsSpawned())
                .append(" th=").append(bits(r.getTotalHealth()))
                .append(" alive=").append(r.getTotalRaidersAlive())
                .append(" c=").append(r.getCenter().getX()).append(",").append(r.getCenter().getY()).append(",").append(r.getCenter().getZ())
                .append(" wp=").append(wave.map(p -> p.getX() + "," + p.getY() + "," + p.getZ()).orElse("-"))
                .append(" cel=").append(celebrationTicks.getInt(r))
                .append(" lead=").append(new TreeSet<>(leaders.keySet()))
                .append(" bar=").append(bits(bar.getProgress())).append(",").append(name).append(",").append(bar.isVisible() ? 1 : 0)
                .append(",").append(player != null && bar.getPlayers().contains(player) ? 1 : 0);
        }
        // Ticks active, cooldown and post-raid ticks through the codec (no getters).
        for (var entry : map.int2ObjectEntrySet()) {
            CompoundTag t = (CompoundTag) Raid.MAP_CODEC.codec().encodeStart(NbtOps.INSTANCE, entry.getValue()).getOrThrow();
            s.append(" ; t").append(entry.getIntKey()).append(" ").append(t.getLongOr("ticks_active", -1)).append(" ")
                .append(t.getIntOr("cooldown_ticks", -1)).append(" ").append(t.getIntOr("post_raid_ticks", -1));
        }
        return s.toString();
    }

    /** NBT as canonical text: sorted compound keys, exact numbers. */
    static String canon(Tag tag) {
        return switch (tag) {
            case ByteTag b -> b.value() + "b";
            case ShortTag v -> v.value() + "s";
            case IntTag v -> Integer.toString(v.value());
            case LongTag v -> v.value() + "L";
            case FloatTag v -> "f" + bits(v.value());
            case DoubleTag v -> "d" + bits(v.value());
            case StringTag v -> "\"" + v.value() + "\"";
            case ByteArrayTag v -> { StringBuilder s = new StringBuilder("[B;"); for (byte x : v.getAsByteArray()) s.append(x).append(","); yield s + "]"; }
            case IntArrayTag v -> { StringBuilder s = new StringBuilder("[I;"); for (int x : v.getAsIntArray()) s.append(x).append(","); yield s + "]"; }
            case LongArrayTag v -> { StringBuilder s = new StringBuilder("[L;"); for (long x : v.getAsLongArray()) s.append(x).append(","); yield s + "]"; }
            case ListTag v -> { StringBuilder s = new StringBuilder("["); for (Tag x : v) s.append(canon(x)).append(","); yield s + "]"; }
            case CompoundTag v -> {
                StringBuilder s = new StringBuilder("{");
                for (String k : new TreeSet<>(v.keySet())) s.append(k).append(":").append(canon(v.get(k))).append(",");
                yield s + "}";
            }
            default -> "?";
        };
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        putObject = unsafe.getClass().getMethod("putObject", Object.class, long.class, Object.class);
        objectFieldOffset = unsafe.getClass().getMethod("objectFieldOffset", Field.class);
        var repo = ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(List.of("vanilla"));
        var resources = new MultiPackResourceManager(PackType.SERVER_DATA, repo.openAllSelected());
        var layers = RegistryLayer.createRegistryAccess();
        var staticTags = TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(RegistryLayer.STATIC));
        var context = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), staticTags);
        RegistryAccess.Frozen world = RegistryDataLoader.load(resources, context, RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        var worldLayers = layers.replaceFrom(RegistryLayer.WORLD, world);
        var loaded = net.minecraft.server.ReloadableServerRegistries.reload(worldLayers, staticTags, resources, Runnable::run).join();
        HolderLookup.Provider lookup = loaded.lookupWithUpdatedTags();
        var components = BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(lookup);
        staticTags.forEach(Registry.PendingTags::apply);
        components.forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
        RegistryAccess access = loaded.layers().compositeAccess();
        var biomes = access.lookupOrThrow(Registries.BIOME);
        var dimensions = access.lookupOrThrow(Registries.DIMENSION_TYPE);
        var blockLookup = access.lookupOrThrow(Registries.BLOCK);

        isPatrolling = PatrollingMonster.class.getDeclaredMethod("isPatrolling");
        isPatrolling.setAccessible(true);
        raidMap = Raids.class.getDeclaredField("raidMap"); raidMap.setAccessible(true);
        raidRandom = Raid.class.getDeclaredField("random"); raidRandom.setAccessible(true);
        raidEvent = Raid.class.getDeclaredField("raidEvent"); raidEvent.setAccessible(true);
        celebrationTicks = Raid.class.getDeclaredField("celebrationTicks"); celebrationTicks.setAccessible(true);
        waveSpawnPos = Raid.class.getDeclaredField("waveSpawnPos"); waveSpawnPos.setAccessible(true);
        groupToLeader = Raid.class.getDeclaredField("groupToLeaderMap"); groupToLeader.setAccessible(true);
        noActionTime = LivingEntity.class.getDeclaredField("noActionTime"); noActionTime.setAccessible(true);
        removalReason = Entity.class.getDeclaredField("removalReason"); removalReason.setAccessible(true);

        var server = (net.minecraft.server.MinecraftServer) alloc(net.minecraft.server.dedicated.DedicatedServer.class);
        put(server, net.minecraft.server.MinecraftServer.class, "debugSubscribers", alloc(net.minecraft.util.debug.ServerDebugSubscribers.class));
        Lvl level = (Lvl) alloc(Lvl.class);
        level.fake = server;
        put(level, ServerLevel.class, "server", server);
        level.registries = access;
        level.blocks = new ArrayList<>();
        level.lights = new ArrayList<>();
        level.unloaded = new HashSet<>();
        level.noTicking = new HashSet<>();
        level.villages = new ArrayList<>();
        level.pois = new ArrayList<>();
        level.entities = new ArrayList<>();
        level.playerList = new ArrayList<>();
        level.neutral = new Neutral();
        level.border = new net.minecraft.world.level.border.WorldBorder();
        level.biomeManager = new BiomeManager((x, y, z) -> level.biome, 0L) {
            @Override public Holder<Biome> getBiome(BlockPos pos) { return level.biome; }
            @Override public Holder<Biome> getBiome(int x, int y, int z) { return level.biome; }
            @Override public Holder<Biome> getNoiseBiomeAtPosition(double x, double y, double z) { return level.biome; }
            @Override public Holder<Biome> getNoiseBiomeAtPosition(BlockPos pos) { return level.biome; }
            @Override public Holder<Biome> getNoiseBiomeAtQuart(int x, int y, int z) { return level.biome; }
        };
        // The village points of interest for createOrExtendRaid.
        var poi = (net.minecraft.world.entity.ai.village.poi.PoiManager) alloc(PoiStub.class);
        ((PoiStub) poi).level = level;
        level.poi = poi;
        Field uuidField = Entity.class.getDeclaredField("uuid");
        uuidField.setAccessible(true);
        Field entityId = Entity.class.getDeclaredField("id");
        entityId.setAccessible(true);

        List<String> lines = Files.readAllLines(Path.of(args[0]));
        try (var w = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            for (String raw : lines) {
                if (raw.isBlank()) continue;
                Map<String, String> c = new LinkedHashMap<>();
                for (String token : raw.trim().split("\\s+")) {
                    int eq = token.indexOf('=');
                    if (eq < 0) c.put("name", token); else c.put(token.substring(0, eq), token.substring(eq + 1));
                }
                List<String> log = new ArrayList<>();
                level.log = log;
                try {
                    String mode = c.getOrDefault("mode", "raid");
                    if (mode.equals("codec")) {
                        String snbt = new String(java.util.HexFormat.of().parseHex(c.get("snbt")), StandardCharsets.UTF_8);
                        Tag tag = TagParser.parseCompoundFully(snbt);
                        Optional<Raids> parsed = Raids.CODEC.parse(NbtOps.INSTANCE, tag).resultOrPartial();
                        Raids raids = parsed.orElseGet(Raids::new);
                        log.add(parsed.isPresent() ? "decoded" : "fresh");
                        log.add(canon(Raids.CODEC.encodeStart(NbtOps.INSTANCE, raids).getOrThrow()));
                        // The decoded raids' bars.
                        @SuppressWarnings("unchecked")
                        var map = (it.unimi.dsi.fastutil.ints.Int2ObjectMap<Raid>) raidMap.get(raids);
                        for (var e : map.int2ObjectEntrySet()) {
                            ServerBossEvent bar = (ServerBossEvent) raidEvent.get(e.getValue());
                            log.add("bar " + e.getIntKey() + " " + bits(bar.getProgress()) + " " + (bar.isVisible() ? 1 : 0) + " " + (raids.isDirty() ? 1 : 0));
                        }
                        w.println(c.get("name") + " " + String.join(" ; ", log));
                        continue;
                    }
                    level.dimensionKey = ResourceKey.create(Registries.DIMENSION, Identifier.parse(c.getOrDefault("dim", "minecraft:overworld")));
                    level.dimensionType = dimensions.getOrThrow(ResourceKey.create(Registries.DIMENSION_TYPE, level.dimensionKey.identifier())).value();
                    level.blocks.clear();
                    for (int i = 0; c.containsKey("k" + i); i++) {
                        String[] p = c.get("k" + i).split("/", 2);
                        level.blocks.add(Map.entry(Box.parse(p[0]), net.minecraft.commands.arguments.blocks.BlockStateParser.parseForBlock(blockLookup, p[1], false).blockState()));
                    }
                    level.lights.clear();
                    for (int i = 0; c.containsKey("l" + i); i++) {
                        String[] p = c.get("l" + i).split("/", 2);
                        String[] v = p[1].split(",");
                        level.lights.add(Map.entry(Box.parse(p[0]), new int[] {Integer.parseInt(v[0]), Integer.parseInt(v[1])}));
                    }
                    level.biome = biomes.getOrThrow(ResourceKey.create(Registries.BIOME, Identifier.parse(c.getOrDefault("biome", "minecraft:plains"))));
                    level.sky = Integer.parseInt(c.getOrDefault("sky", "15"));
                    level.block = Integer.parseInt(c.getOrDefault("light", "0"));
                    level.darken = Integer.parseInt(c.getOrDefault("darken", "0"));
                    level.difficulty = Difficulty.byName(c.getOrDefault("difficulty", "normal"));
                    level.clock = Long.parseLong(c.getOrDefault("clock", "200000"));
                    level.inhabited = Long.parseLong(c.getOrDefault("inhabited", "0"));
                    level.moon = Float.parseFloat(c.getOrDefault("moon", "0.0"));
                    level.rules = new GameRules(net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags());
                    level.rules.set(GameRules.SPAWN_PATROLS, !c.getOrDefault("patrols", "1").equals("0"), null);
                    level.rules.set(GameRules.RAIDS, !c.getOrDefault("raids", "1").equals("0"), null);
                    var clocks = (net.minecraft.world.clock.ClockManager) def -> new net.minecraft.world.clock.ClockInstance() {
                        @Override public long totalTicks() { return level.clock; }
                        @Override public float partialTick() { return 0.0F; }
                        @Override public float rate() { return 1.0F; }
                        @Override public boolean isPaused() { return false; }
                    };
                    var attrs = net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().addStaticLayers(level);
                    level.dimensionType.timelines().forEach(t -> attrs.addTimelineLayer(t, clocks));
                    level.attributes = attrs.build();
                    level.unloaded.clear();
                    for (String key : new String[] {"unloaded", "noticking"}) {
                        if (!c.containsKey(key)) continue;
                        for (String u : c.get(key).split(";")) {
                            String[] p = u.split(",");
                            (key.equals("unloaded") ? level.unloaded : level.noTicking).add(ChunkPos.pack(Integer.parseInt(p[0]), Integer.parseInt(p[1])));
                        }
                    }
                    if (!c.containsKey("noticking")) level.noTicking.clear();
                    level.villages.clear();
                    if (c.containsKey("vil")) for (String v : c.get("vil").split(";")) {
                        String[] p = v.split(",");
                        level.villages.add(new int[] {Integer.parseInt(p[0]), Integer.parseInt(p[1]), Integer.parseInt(p[2])});
                    }
                    level.pois.clear();
                    if (c.containsKey("poi")) for (String v : c.get("poi").split(";")) {
                        String[] p = v.split(",");
                        level.pois.add(new BlockPos(Integer.parseInt(p[0]), Integer.parseInt(p[1]), Integer.parseInt(p[2])));
                    }
                    level.entities.clear();
                    level.next = 1;
                    level.playerList.clear();
                    level.finalizing = false;
                    level.grouped = false;
                    level.main = RandomSource.create(Long.parseLong(c.getOrDefault("seed", "0")));
                    put(level, Level.class, "random", level.main);
                    Raids raids = new Raids();
                    put(level, ServerLevel.class, "raids", raids);
                    OraclePlayer player = null;
                    Conn conn = (Conn) alloc(Conn.class);
                    conn.log = log;
                    if (c.containsKey("p0")) {
                        String[] p = c.get("p0").split("/");
                        String[] v = p[0].split(",");
                        player = (OraclePlayer) alloc(OraclePlayer.class);
                        player.mode = GameType.byName(p.length > 1 ? p[1] : "survival");
                        player.alive = true;
                        player.log = log;
                        player.adv = (PlayerAdvancements) alloc(Adv.class);
                        player.connection = conn;
                        entityId.setInt(player, 100000);
                        uuidField.set(player, new UUID(0x1234L, 0x5678L));
                        put(player, Entity.class, "level", level);
                        setPosition(player, Double.parseDouble(v[0]), Double.parseDouble(v[1]), Double.parseDouble(v[2]));
                        level.playerList.add(player);
                    }
                    int ticks = Integer.parseInt(c.getOrDefault("ticks", "1"));
                    if (mode.equals("patrol")) {
                        PatrolSpawner spawner = new PatrolSpawner();
                        Field next = PatrolSpawner.class.getDeclaredField("nextTick");
                        next.setAccessible(true);
                        next.setInt(spawner, Integer.parseInt(c.getOrDefault("next", "0")));
                        boolean enemies = !c.getOrDefault("enemies", "1").equals("0");
                        for (int t = 0; t < ticks; t++) {
                            level.grouped = false;
                            level.finalizing = false;
                            spawner.tick(level, enemies);
                        }
                        log.add("next " + next.getInt(spawner) + " " + level.main.nextInt(1000));
                        w.println(c.get("name") + " " + String.join(" ; ", log));
                        continue;
                    }
                    // raid: the first omen makes the raid at its position (village centre from the POIs).
                    level.main = level.neutral;
                    put(level, Level.class, "random", level.neutral);
                    level.finalizing = true;
                    long seed = Long.parseLong(c.getOrDefault("rseed", "1"));
                    List<Raider> tracked = new ArrayList<>();
                    Set<Raid> seeded = Collections.newSetFromMap(new IdentityHashMap<>());
                    Map<String, List<String>> events = new TreeMap<>();
                    for (int i = 0; c.containsKey("ev" + i); i++) {
                        String[] p = c.get("ev" + i).split(":", 2);
                        events.computeIfAbsent(p[0], k -> new ArrayList<>()).add(p[1]);
                    }
                    String lastState = "";
                    for (int t = 0; t <= ticks; t++) {
                        int before = level.entities.size();
                        for (String ev : events.getOrDefault(Integer.toString(t), List.of())) {
                            String[] a = ev.split(":");
                            log.add("ev " + ev);
                            if (Set.of("hurt", "kill", "move", "age", "idle", "gone", "discard").contains(a[0]) && Integer.parseInt(a[1]) >= tracked.size()) {
                                log.add("noraider");
                                continue;
                            }
                            if (a[0].equals("killall") || a[0].equals("hurtall")) {
                                for (int i = 0; i < tracked.size(); i++) {
                                    Raider r = tracked.get(i);
                                    if (r.getCurrentRaid() == null || r.isRemoved()) continue;
                                    if (a[0].equals("hurtall")) {
                                        if (r.hasActiveRaid()) r.getCurrentRaid().updateBossbar();
                                        r.setHealth(r.getHealth() - Float.parseFloat(a[1]));
                                        continue;
                                    }
                                    Raid raid = r.getCurrentRaid();
                                    if (r.isPatrolLeader()) raid.removeLeader(r.getWave());
                                    if (a.length > 1 && player != null) raid.addHeroOfTheVillage(player);
                                    raid.removeFromRaid(level, r, false);
                                    r.setHealth(0.0F);
                                    removalReason.set(r, Entity.RemovalReason.KILLED);
                                    level.entities.remove(r);
                                }
                                continue;
                            }
                            switch (a[0]) {
                                case "omen" -> {
                                    if (player == null) throw new IllegalStateException("omen needs p0");
                                    String[] v = a[2].split(",");
                                    setPosition(player, Integer.parseInt(v[0]) + 0.5, Integer.parseInt(v[1]), Integer.parseInt(v[2]) + 0.5);
                                    player.omen = Integer.parseInt(a[1]);
                                    Raid r = raids.createOrExtendRaid(player, player.blockPosition());
                                    player.omen = -1;
                                    if (r == null) log.add("noraid");
                                    else if (seeded.add(r)) {
                                        int id = raids.getId(r).orElse(-1);
                                        raidRandom.set(r, RandomSource.create(seed + id * 1000L));
                                    }
                                }
                                case "setomen" -> {
                                    Raid r = level.getRaidAt(player.blockPosition());
                                    if (r != null) r.setRaidOmenLevel(Integer.parseInt(a[1]));
                                }
                                case "stop" -> {
                                    Raid r = level.getRaidAt(player.blockPosition());
                                    if (r != null) r.stop();
                                }
                                case "pmove" -> {
                                    String[] v = a[1].split(",");
                                    setPosition(player, Double.parseDouble(v[0]), Double.parseDouble(v[1]), Double.parseDouble(v[2]));
                                }
                                case "pdead" -> player.alive = false;
                                case "palive" -> player.alive = true;
                                case "diff" -> level.difficulty = Difficulty.byName(a[1]);
                                case "rules" -> level.rules.set(GameRules.RAIDS, a[1].equals("1"), null);
                                case "unload" -> level.unloaded.add(chunk(a[1]));
                                case "load" -> level.unloaded.remove(chunk(a[1]));
                                case "notick" -> level.noTicking.add(chunk(a[1]));
                                case "tick" -> level.noTicking.remove(chunk(a[1]));
                                case "vil" -> level.villages.add(ints(a[1]));
                                case "unvil" -> level.villages.removeIf(v -> Arrays.equals(v, ints(a[1])));
                                case "hurt" -> {
                                    Raider r = tracked.get(Integer.parseInt(a[1]));
                                    if (r.hasActiveRaid()) r.getCurrentRaid().updateBossbar();
                                    r.setHealth(r.getHealth() - Float.parseFloat(a[2]));
                                }
                                case "kill" -> {
                                    Raider r = tracked.get(Integer.parseInt(a[1]));
                                    Raid raid = r.getCurrentRaid();
                                    if (raid != null) {
                                        if (r.isPatrolLeader()) raid.removeLeader(r.getWave());
                                        if (a.length > 2 && player != null) raid.addHeroOfTheVillage(player);
                                        raid.removeFromRaid(level, r, false);
                                    }
                                    r.setHealth(0.0F);
                                    removalReason.set(r, Entity.RemovalReason.KILLED);
                                    level.entities.remove(r);
                                }
                                case "move" -> {
                                    String[] v = a[2].split(",");
                                    tracked.get(Integer.parseInt(a[1])).setPos(Integer.parseInt(v[0]) + 0.5, Integer.parseInt(v[1]), Integer.parseInt(v[2]) + 0.5);
                                }
                                case "age" -> tracked.get(Integer.parseInt(a[1])).tickCount = Integer.parseInt(a[2]);
                                case "idle" -> noActionTime.setInt(tracked.get(Integer.parseInt(a[1])), Integer.parseInt(a[2]));
                                case "gone" -> level.entities.remove(tracked.get(Integer.parseInt(a[1])));
                                case "discard" -> removalReason.set(tracked.get(Integer.parseInt(a[1])), Entity.RemovalReason.DISCARDED);
                                case "join", "loadmob" -> {
                                    EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(a[1]));
                                    Raider r = (Raider) type.create(level, EntitySpawnReason.LOAD);
                                    String[] v = a[2].split(",");
                                    r.setPos(Integer.parseInt(v[0]) + 0.5, Integer.parseInt(v[1]), Integer.parseInt(v[2]) + 0.5);
                                    level.entities.add(r);
                                    tracked.add(r);
                                    if (a[0].equals("join")) {
                                        r.setCanJoinRaid(true);
                                        Raid near = level.getRaidAt(r.blockPosition());
                                        if (near != null && Raids.canJoinRaid(r)) near.joinRaid(level, near.getGroupsSpawned(), r, null, true);
                                    } else {
                                        // readAdditionalSaveData: wave, leader, then the raid by id.
                                        int wave = Integer.parseInt(a[3]);
                                        r.setWave(wave);
                                        if (a.length > 4) r.setPatrolLeader(true);
                                        Raid raid = raids.get(Integer.parseInt(a.length > 5 ? a[5] : "2"));
                                        r.setCurrentRaid(raid);
                                        if (raid != null) {
                                            raid.addWaveMob(level, wave, r, false);
                                            if (r.isPatrolLeader()) raid.setLeader(wave, r);
                                        }
                                    }
                                }
                                default -> throw new IllegalArgumentException("event " + a[0]);
                            }
                        }
                        if (t == 0) continue;
                        Map<Raider, Raid> raidOf = new IdentityHashMap<>();
                        Map<Raider, Integer> outside = new IdentityHashMap<>();
                        for (Raider r : tracked) { raidOf.put(r, r.getCurrentRaid()); outside.put(r, r.getTicksOutsideRaid()); }
                        before = level.entities.size();
                        raids.tick(level);
                        for (Entity e : level.entities.subList(before, level.entities.size())) if (e instanceof Raider r) tracked.add(r);
                        for (int i = 0; i < tracked.size(); i++) {
                            Raider r = tracked.get(i);
                            if (raidOf.containsKey(r) && raidOf.get(r) != null && r.getCurrentRaid() == null) log.add("leave " + i);
                            if (outside.containsKey(r) && outside.get(r) != r.getTicksOutsideRaid()) log.add("out " + i + " " + r.getTicksOutsideRaid());
                        }
                        String state = raidState(raids, player);
                        String key = state.replaceAll(" ; t\\d+ [^;]*", "");
                        if (!key.equals(lastState) || t % 1000 == 0 || t == ticks) log.add("t" + t + state);
                        lastState = key;
                    }
                    w.println(c.get("name") + " " + String.join(" ; ", log));
                } catch (Throwable t) {
                    Throwable cause = t;
                    while ((cause instanceof InvocationTargetException || cause instanceof RuntimeException && cause.getCause() != null) && cause.getCause() != null) cause = cause.getCause();
                    StringBuilder line = new StringBuilder(c.get("name") + " error " + cause);
                    if (System.getenv("ORACLE_TRACE") != null) for (var frame : cause.getStackTrace()) line.append("\n  at ").append(frame);
                    w.println(line);
                }
            }
        }
        System.exit(0);
    }

    static long chunk(String s) { String[] p = s.split(","); return ChunkPos.pack(Integer.parseInt(p[0]), Integer.parseInt(p[1])); }
    static int[] ints(String s) { return Arrays.stream(s.split(",")).mapToInt(Integer::parseInt).toArray(); }

    static void setPosition(Entity e, double x, double y, double z) throws Exception {
        put(e, Entity.class, "position", new Vec3(x, y, z));
        put(e, Entity.class, "blockPosition", BlockPos.containing(x, y, z));
    }

    /** The village points of interest: all occupied, in the order given. */
    static final class PoiStub extends net.minecraft.world.entity.ai.village.poi.PoiManager {
        PoiStub() { super(null, null, null, false, null, null, null); }
        Lvl level;
        @Override public java.util.stream.Stream<net.minecraft.world.entity.ai.village.poi.PoiRecord> getInSquare(
            Predicate<Holder<net.minecraft.world.entity.ai.village.poi.PoiType>> predicate, BlockPos center, int radius,
            net.minecraft.world.entity.ai.village.poi.PoiManager.Occupancy occupancy) {
            var type = BuiltInRegistries.POINT_OF_INTEREST_TYPE.getOrThrow(net.minecraft.world.entity.ai.village.poi.PoiTypes.MEETING);
            List<net.minecraft.world.entity.ai.village.poi.PoiRecord> out = new ArrayList<>();
            for (BlockPos p : level.pois)
                if (Math.abs(p.getX() - center.getX()) <= radius && Math.abs(p.getZ() - center.getZ()) <= radius)
                    out.add(new net.minecraft.world.entity.ai.village.poi.PoiRecord(p, type, () -> {}));
            return out.stream();
        }
    }
}
