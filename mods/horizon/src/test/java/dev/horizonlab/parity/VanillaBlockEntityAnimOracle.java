package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.function.Predicate;
import net.minecraft.client.model.object.book.BookModel;
import net.minecraft.client.model.object.chest.ChestModel;
import net.minecraft.client.renderer.blockentity.*;
import net.minecraft.client.renderer.blockentity.state.*;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.*;
import net.minecraft.core.particles.ColorParticleOption;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.core.particles.ParticleTypes;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.sounds.SoundSource;
import net.minecraft.util.RandomSource;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.entity.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.*;

/**
 * Original oracle for the client-ticked animation state of chests and ender
 * chests (ChestLidController), enchanting table books, shulker boxes, bells,
 * conduits, skulls, decorated pots and End gateways. Each case builds the real
 * vanilla block entity on a stand-in client level (scripted blocks, entities,
 * game time and a seeded LegacyRandomSource), then per step applies the
 * scripted block event (triggerEvent), runs the block entity's own client
 * ticker (lidAnimateTick, bookAnimationTick with the class's static RANDOM
 * reseeded, ShulkerBoxBlockEntity.tick, BellBlockEntity.clientTick,
 * ConduitBlockEntity.clientTick, SkullBlockEntity.animation,
 * TheEndGatewayBlockEntity.beamAnimationTick after the server copy's portalTick
 * on a server-side stand-in; pots have none)
 * and runs the real renderer's extractRenderState at the step's partial tick.
 * It prints the block entity's fields, the level calls the tick made
 * (neighbour updates, entity queries, sounds, particles) and the render state.
 *
 * Usage: VanillaBlockEntityAnimOracle inputs.jsonl output.txt
 */
public final class VanillaBlockEntityAnimOracle {
    static Object unsafe;
    static Method allocate, putObject, objectFieldOffset;

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) throws Exception { return (T) allocate.invoke(unsafe, type); }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        long offset = (long) objectFieldOffset.invoke(unsafe, f);
        putObject.invoke(unsafe, target, offset, value);
    }

    static Field field(Class<?> owner, String name) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static String f(float v) { return String.format("%08x", Float.floatToRawIntBits(v)); }
    static String d(double v) { return String.format("%016x", Double.doubleToRawLongBits(v)); }
    static String box(AABB b) { return d(b.minX) + " " + d(b.minY) + " " + d(b.minZ) + " " + d(b.maxX) + " " + d(b.maxY) + " " + d(b.maxZ); }

    static BlockState parse(String s) throws Exception {
        return BlockStateParser.parseForBlock(BuiltInRegistries.BLOCK, s, false).blockState();
    }

    /** A living entity the case places: a Pillager whose type field says raider or not. */
    static final class Mob extends net.minecraft.world.entity.monster.illager.Pillager {
        boolean living;
        Mob() { super(null, null); }
        @Override public boolean isAlive() { return !this.isRemoved() && this.living; }
    }

    /** A player the case places: position and game mode (spectators are skipped). */
    static final class Gamer extends Player {
        GameType mode;
        Gamer() { super(null, null); }
        @Override public GameType gameMode() { return mode; }
        @Override public net.minecraft.world.item.component.ResolvableProfile getProfile() { return null; }
    }

    static Gamer gamer(JsonObject e) throws Exception {
        Gamer g = make(Gamer.class);
        g.mode = e.has("spectator") && e.get("spectator").getAsBoolean() ? GameType.SPECTATOR : GameType.SURVIVAL;
        var p = e.getAsJsonArray("pos");
        put(g, Entity.class, "position", new Vec3(p.get(0).getAsDouble(), p.get(1).getAsDouble(), p.get(2).getAsDouble()));
        return g;
    }

    /** The stand-in client level. */
    static final class World extends Level {
        World() { super(null, null, null, null, false, false, 0L, 0); }
        long time;
        boolean server;
        int minY, height;
        BlockEntity serverCopyOf;
        Map<BlockPos, BlockState> blocks = new HashMap<>();
        List<Mob> mobs = new ArrayList<>();
        List<Player> gamers = new ArrayList<>();
        Map<BlockPos, BlockEntity> entities = new HashMap<>();
        RandomSource random;
        StringBuilder log = new StringBuilder();

        @Override public long getGameTime() { return time; }
        @Override public boolean isClientSide() { return !server; }
        @Override public int getMinY() { return minY; }
        @Override public int getHeight() { return height; }
        @Override public void blockEvent(BlockPos p, Block block, int b0, int b1) {
            // The integrated server's block event reaching the client copy.
            log.append(" blockEvent ").append(b0).append(' ').append(b1);
            serverCopyOf.triggerEvent(b0, b1);
        }
        @Override public void blockEntityChanged(BlockPos p) {}
        @Override public void updateNeighbourForOutputSignal(BlockPos p, Block block) {}
        @Override public BlockState getBlockState(BlockPos p) { return blocks.getOrDefault(p.immutable(), Blocks.AIR.defaultBlockState()); }
        @Override public FluidState getFluidState(BlockPos p) { return getBlockState(p).getFluidState(); }
        @Override public int getBrightness(LightLayer layer, BlockPos p) { return 0; }
        @Override public RandomSource getRandom() { return random; }
        @Override public BlockEntity getBlockEntity(BlockPos p) { return entities.get(p.immutable()); }
        @Override public void updateNeighborsAt(BlockPos p, Block block) { log.append(" neighbors"); }
        @Override public void neighborShapeChanged(Direction dir, BlockPos p, BlockPos from, BlockState state, int flags, int limit) {}
        @Override public List<Entity> getEntities(Entity except, AABB bb) { log.append(" push ").append(box(bb)); return List.of(); }
        @SuppressWarnings("unchecked")
        @Override public <T extends Entity> List<T> getEntitiesOfClass(Class<T> type, AABB bb, Predicate<? super T> selector) {
            log.append(" gather ").append(box(bb));
            List<T> out = new ArrayList<>();
            for (Mob m : mobs) if (type.isInstance(m) && bb.contains(m.position())) out.add((T) m);
            return out;
        }
        @Override public void playSound(Entity except, BlockPos p, SoundEvent s, SoundSource source, float volume, float pitch) {
            log.append(" sound ").append(BuiltInRegistries.SOUND_EVENT.getKey(s));
        }
        @Override public void addParticle(ParticleOptions o, double x, double y, double z, double xd, double yd, double zd) {
            log.append(" particle ").append(BuiltInRegistries.PARTICLE_TYPE.getKey(o.getType()));
            if (o instanceof ColorParticleOption c) {
                try { log.append(" ").append(field(ColorParticleOption.class, "color").getInt(c)); } catch (Exception e) { throw new RuntimeException(e); }
            }
            log.append(" ").append(d(x)).append(" ").append(d(y)).append(" ").append(d(z))
                .append(" ").append(d(xd)).append(" ").append(d(yd)).append(" ").append(d(zd));
        }
@Override public net.minecraft.world.clock.ClockManager clockManager(){throw new UnsupportedOperationException();}
@Override public void destroyBlockProgress(int p0,net.minecraft.core.BlockPos p1,int p2){throw new UnsupportedOperationException();}
@Override public java.util.Collection dragonParts(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes(){throw new UnsupportedOperationException();}
@Override public void explode(net.minecraft.world.entity.Entity p0,net.minecraft.world.damagesource.DamageSource p1,net.minecraft.world.level.ExplosionDamageCalculator p2,double p3,double p4,double p5,float p6,boolean p7,net.minecraft.world.level.Level.ExplosionInteraction p8,net.minecraft.core.particles.ParticleOptions p9,net.minecraft.core.particles.ParticleOptions p10,net.minecraft.util.random.WeightedList p11,net.minecraft.core.Holder p12){throw new UnsupportedOperationException();}
@Override public void gameEvent(net.minecraft.core.Holder p0,net.minecraft.world.phys.Vec3 p1,net.minecraft.world.level.gameevent.GameEvent.Context p2){throw new UnsupportedOperationException();}
@Override public java.lang.String gatherChunkSourceStats(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.ticks.LevelTickAccess getBlockTicks(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.chunk.ChunkSource getChunkSource(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.entity.LevelEntityGetter getEntities(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.entity.Entity getEntity(int p0){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.ticks.LevelTickAccess getFluidTicks(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.saveddata.maps.MapItemSavedData getMapData(net.minecraft.world.level.saveddata.maps.MapId p0){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.storage.LevelData.RespawnData getRespawnData(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.scores.Scoreboard getScoreboard(){throw new UnsupportedOperationException();}
@Override public int getSeaLevel(){throw new UnsupportedOperationException();}
@Override public net.minecraft.core.Holder getUncachedNoiseBiome(int p0,int p1,int p2){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.border.WorldBorder getWorldBorder(){throw new UnsupportedOperationException();}
@Override public void levelEvent(net.minecraft.world.entity.Entity p0,int p1,net.minecraft.core.BlockPos p2,int p3){throw new UnsupportedOperationException();}
@Override public void playSeededSound(net.minecraft.world.entity.Entity p0,net.minecraft.world.entity.Entity p1,net.minecraft.core.Holder p2,net.minecraft.sounds.SoundSource p3,float p4,float p5,long p6){throw new UnsupportedOperationException();}
@Override public void playSeededSound(net.minecraft.world.entity.Entity p0,double p1,double p2,double p3,net.minecraft.core.Holder p4,net.minecraft.sounds.SoundSource p5,float p6,float p7,long p8){throw new UnsupportedOperationException();}
@Override public java.util.List players(){return gamers;}
@Override public net.minecraft.world.item.crafting.RecipeAccess recipeAccess(){throw new UnsupportedOperationException();}
@Override public void sendBlockUpdated(net.minecraft.core.BlockPos p0,net.minecraft.world.level.block.state.BlockState p1,net.minecraft.world.level.block.state.BlockState p2,int p3){throw new UnsupportedOperationException();}
@Override public void setRespawnData(net.minecraft.world.level.storage.LevelData.RespawnData p0){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.TickRateManager tickRateManager(){throw new UnsupportedOperationException();}
    }

    static Mob mob(JsonObject e, World world) throws Exception {
        Mob m = make(Mob.class);
        m.living = !e.has("alive") || e.get("alive").getAsBoolean();
        put(m, Entity.class, "type", e.has("raider") && e.get("raider").getAsBoolean() ? EntityTypes.PILLAGER : EntityTypes.COW);
        var p = e.getAsJsonArray("pos");
        put(m, Entity.class, "position", new Vec3(p.get(0).getAsDouble(), p.get(1).getAsDouble(), p.get(2).getAsDouble()));
        if (e.has("removed") && e.get("removed").getAsBoolean()) put(m, Entity.class, "removalReason", Entity.RemovalReason.KILLED);
        if (e.has("width")) {
            put(m, Entity.class, "dimensions", EntityDimensions.scalable(e.get("width").getAsFloat(), e.get("height").getAsFloat()));
            Field eye = field(Entity.class, "eyeHeight");
            eye.setFloat(m, e.get("eye").getAsFloat());
        }
        return m;
    }

    static ChestModel CHEST_MODEL;

    static void tickLid(World world, BlockEntity e) {
        if (e instanceof ChestBlockEntity c) ChestBlockEntity.lidAnimateTick(world, c.getBlockPos(), c.getBlockState(), c);
        else if (e instanceof EnderChestBlockEntity c) EnderChestBlockEntity.lidAnimateTick(world, c.getBlockPos(), c.getBlockState(), c);
    }

    static String lid(BlockEntity e) throws Exception {
        if (!(e instanceof ChestBlockEntity) && !(e instanceof EnderChestBlockEntity)) return "-";
        Class<?> owner = e instanceof ChestBlockEntity ? ChestBlockEntity.class : EnderChestBlockEntity.class;
        Object lid = field(owner, "chestLidController").get(e);
        return field(ChestLidController.class, "shouldBeOpen").getBoolean(lid)
            + " " + f(field(ChestLidController.class, "openness").getFloat(lid))
            + " " + f(field(ChestLidController.class, "oOpenness").getFloat(lid));
    }

    static String run(JsonObject c) throws Exception {
        World world = make(World.class);
        world.blocks = new HashMap<>();
        world.mobs = new ArrayList<>();
        world.gamers = new ArrayList<>();
        world.entities = new HashMap<>();
        world.log = new StringBuilder();
        world.random = RandomSource.create(c.has("seed") ? c.get("seed").getAsLong() : 0L);
        var pa = c.getAsJsonArray("pos");
        BlockPos pos = new BlockPos(pa.get(0).getAsInt(), pa.get(1).getAsInt(), pa.get(2).getAsInt());
        BlockState state = parse(c.get("block").getAsString());
        world.blocks.put(pos, state);
        if (c.has("world")) {
            for (var e : c.getAsJsonObject("world").entrySet()) {
                String[] xyz = e.getKey().split(",");
                world.blocks.put(pos.offset(Integer.parseInt(xyz[0]), Integer.parseInt(xyz[1]), Integer.parseInt(xyz[2])), parse(e.getValue().getAsString()));
            }
        }
        if (c.has("entities")) for (var e : c.getAsJsonArray("entities")) world.mobs.add(mob(e.getAsJsonObject(), world));
        var be = ((EntityBlock) state.getBlock()).newBlockEntity(pos, state);
        be.setLevel(world);
        world.entities.put(pos, be);
        // The other half of a double chest (or whatever block sits there).
        BlockEntity partner = null;
        if (c.has("partner")) {
            String[] xyz = c.get("partner").getAsString().split(",");
            BlockPos at = pos.offset(Integer.parseInt(xyz[0]), Integer.parseInt(xyz[1]), Integer.parseInt(xyz[2]));
            BlockState there = world.getBlockState(at);
            if (there.getBlock() instanceof EntityBlock eb) {
                partner = eb.newBlockEntity(at, there);
                partner.setLevel(world);
                world.entities.put(at, partner);
            }
        }
        if (c.has("players")) for (var e : c.getAsJsonArray("players")) world.gamers.add(gamer(e.getAsJsonObject()));
        if (be instanceof EnchantingTableBlockEntity) {
            ((RandomSource) field(EnchantingTableBlockEntity.class, "RANDOM").get(null)).setSeed(c.get("seed").getAsLong());
        }
        if (c.has("target") && be instanceof ConduitBlockEntity) {
            put(be, ConduitBlockEntity.class, "destroyTarget", EntityReference.of(mob(c.getAsJsonObject("target"), world)));
        }
        World serverWorld = null;
        TheEndGatewayBlockEntity serverGateway = null;
        if (be instanceof TheEndGatewayBlockEntity g) {
            var level = c.getAsJsonArray("level");
            world.minY = level.get(0).getAsInt();
            world.height = level.get(1).getAsInt();
            field(TheEndGatewayBlockEntity.class, "age").setLong(g, c.get("age").getAsLong());
            serverWorld = make(World.class);
            serverWorld.server = true;
            serverWorld.blocks = world.blocks;
            serverWorld.log = world.log;
            serverWorld.serverCopyOf = g;
            serverGateway = (TheEndGatewayBlockEntity) ((EntityBlock) state.getBlock()).newBlockEntity(pos, state);
            serverGateway.setLevel(serverWorld);
            field(TheEndGatewayBlockEntity.class, "age").setLong(serverGateway, c.get("age").getAsLong());
        }
        StringBuilder out = new StringBuilder();
        for (var stepElement : c.getAsJsonArray("steps")) {
            var step = stepElement.getAsJsonObject();
            world.time = step.get("time").getAsLong();
            world.log.setLength(0);
            if (step.has("set")) {
                for (var e : step.getAsJsonObject("set").entrySet()) {
                    String[] xyz = e.getKey().split(",");
                    world.blocks.put(pos.offset(Integer.parseInt(xyz[0]), Integer.parseInt(xyz[1]), Integer.parseInt(xyz[2])), parse(e.getValue().getAsString()));
                }
            }
            if (step.has("players")) {
                world.gamers.clear();
                for (var e : step.getAsJsonArray("players")) world.gamers.add(gamer(e.getAsJsonObject()));
            }
            if (serverGateway != null) {
                serverWorld.time = world.time;
                // ServerLevel.tick: entities (EndGatewayBlock.entityInside), then block entities.
                if (step.has("enter") && !serverGateway.isCoolingDown()) {
                    TheEndGatewayBlockEntity.triggerCooldown(serverWorld, pos, state, serverGateway);
                }
                TheEndGatewayBlockEntity.portalTick(serverWorld, pos, state, serverGateway);
            }
            if (step.has("event")) {
                var ev = step.getAsJsonArray("event");
                boolean handled = be.triggerEvent(ev.get(0).getAsInt(), ev.get(1).getAsInt());
                world.log.append(" event ").append(handled);
            }
            if (step.has("partnerEvent") && partner != null) {
                var ev = step.getAsJsonArray("partnerEvent");
                boolean handled = partner.triggerEvent(ev.get(0).getAsInt(), ev.get(1).getAsInt());
                world.log.append(" partner event ").append(handled);
            }
            float partial = step.get("partial").getAsFloat();
            String fields;
            if (be instanceof ChestBlockEntity || be instanceof EnderChestBlockEntity) {
                tickLid(world, be);
                if (partner != null) tickLid(world, partner);
                ChestRenderer<?> renderer = make(ChestRenderer.class);
                var r = new net.minecraft.client.renderer.blockentity.state.ChestRenderState();
                @SuppressWarnings({"unchecked", "rawtypes"})
                ChestRenderer raw = renderer;
                raw.extractRenderState(be, r, partial, Vec3.ZERO, null);
                float open = r.open;
                // ChestRenderer.submit, then ChestModel.setupAnim.
                open = 1.0F - open;
                open = 1.0F - open * open * open;
                CHEST_MODEL.setupAnim(open);
                fields = lid(be) + " partner " + (partner == null ? "none" : lid(partner))
                    + " render " + f(r.open) + " " + f(((net.minecraft.client.model.geom.ModelPart) field(ChestModel.class, "lid").get(CHEST_MODEL)).xRot);
            } else if (be instanceof EnchantingTableBlockEntity t) {
                EnchantingTableBlockEntity.bookAnimationTick(world, pos, state, t);
                var r = new net.minecraft.client.renderer.blockentity.state.EnchantTableRenderState();
                make(EnchantTableRenderer.class).extractRenderState(t, r, partial, Vec3.ZERO, null);
                // EnchantTableRenderer.submit's book state.
                float ff1 = Mth.frac(r.flip + 0.25F) * 1.6F - 0.3F;
                float ff2 = Mth.frac(r.flip + 0.75F) * 1.6F - 0.3F;
                var book = BookModel.State.forAnimation(r.time, Mth.clamp(ff1, 0.0F, 1.0F), Mth.clamp(ff2, 0.0F, 1.0F), r.open);
                fields = t.time + " " + f(t.flip) + " " + f(t.oFlip) + " " + f(t.flipT) + " " + f(t.flipA)
                    + " " + f(t.open) + " " + f(t.oOpen) + " " + f(t.rot) + " " + f(t.oRot) + " " + f(t.tRot)
                    + " render " + f(r.flip) + " " + f(r.open) + " " + f(r.time) + " " + f(r.yRot)
                    + " book " + f(book.openness()) + " " + f(book.pageFlip1()) + " " + f(book.pageFlip2());
            } else if (be instanceof ShulkerBoxBlockEntity s) {
                ShulkerBoxBlockEntity.tick(world, pos, state, s);
                var r = make(ShulkerBoxRenderer.class).createRenderState();
                make(ShulkerBoxRenderer.class).extractRenderState(s, r, partial, Vec3.ZERO, null);
                fields = s.getAnimationStatus() + " " + field(ShulkerBoxBlockEntity.class, "openCount").getInt(s)
                    + " " + f(field(ShulkerBoxBlockEntity.class, "progress").getFloat(s))
                    + " " + f(field(ShulkerBoxBlockEntity.class, "progressOld").getFloat(s))
                    + " render " + f(r.progress);
            } else if (be instanceof BellBlockEntity b) {
                if (!step.has("noTick")) BellBlockEntity.clientTick(world, pos, state, b);
                var r = make(BellRenderer.class).createRenderState();
                make(BellRenderer.class).extractRenderState(b, r, partial, Vec3.ZERO, null);
                fields = b.ticks + " " + b.shaking + " " + b.clickDirection
                    + " " + field(BellBlockEntity.class, "resonating").getBoolean(b)
                    + " " + field(BellBlockEntity.class, "resonationTicks").getInt(b)
                    + " " + field(BellBlockEntity.class, "lastRingTimestamp").getLong(b)
                    + " render " + f(r.ticks) + " " + r.shakeDirection;
            } else if (be instanceof ConduitBlockEntity k) {
                ConduitBlockEntity.clientTick(world, pos, state, k);
                var r = make(ConduitRenderer.class).createRenderState();
                make(ConduitRenderer.class).extractRenderState(k, r, partial, Vec3.ZERO, null);
                @SuppressWarnings("unchecked")
                var effect = (List<BlockPos>) field(ConduitBlockEntity.class, "effectBlocks").get(k);
                fields = k.tickCount + " " + k.isActive() + " " + k.isHunting()
                    + " " + f(field(ConduitBlockEntity.class, "activeRotation").getFloat(k)) + " " + effect.size()
                    + " render " + r.isActive + " " + f(r.activeRotation) + " " + f(r.animTime) + " " + r.animationPhase + " " + r.isHunting;
            } else if (be instanceof SkullBlockEntity k) {
                BlockState now = step.has("state") ? parse(step.get("state").getAsString()) : state;
                SkullBlockEntity.animation(world, pos, now, k);
                fields = field(SkullBlockEntity.class, "isAnimating").getBoolean(k)
                    + " " + field(SkullBlockEntity.class, "animationTickCount").getInt(k)
                    + " render " + f(k.getAnimation(partial));
            } else if (be instanceof DecoratedPotBlockEntity p) {
                var r = make(DecoratedPotRenderer.class).createRenderState();
                make(DecoratedPotRenderer.class).extractRenderState(p, r, partial, Vec3.ZERO, null);
                fields = p.wobbleStartedAtTick + " " + p.lastWobbleStyle
                    + " render " + f(r.wobbleProgress) + " " + r.wobbleStyle;
            } else if (be instanceof TheEndGatewayBlockEntity g) {
                TheEndGatewayBlockEntity.beamAnimationTick(world, pos, state, g);
                var r = make(TheEndGatewayRenderer.class).createRenderState();
                make(TheEndGatewayRenderer.class).extractRenderState(g, r, partial, Vec3.ZERO, null);
                fields = field(TheEndGatewayBlockEntity.class, "age").getLong(g)
                    + " " + field(TheEndGatewayBlockEntity.class, "teleportCooldown").getInt(g)
                    + " server " + field(TheEndGatewayBlockEntity.class, "age").getLong(serverGateway)
                    + " " + field(TheEndGatewayBlockEntity.class, "teleportCooldown").getInt(serverGateway)
                    + " render " + r.height + " " + f(r.scale) + " " + Integer.toHexString(r.color) + " " + f(r.animationTime);
            } else {
                throw new IllegalArgumentException("no animation for " + be);
            }
            out.append(fields).append(" |").append(world.log).append('\n');
        }
        return out.toString();
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var allocator = Class.forName("sun.misc.Unsafe");
        var theUnsafe = allocator.getDeclaredField("theUnsafe");
        theUnsafe.setAccessible(true);
        unsafe = theUnsafe.get(null);
        allocate = allocator.getMethod("allocateInstance", Class.class);
        putObject = allocator.getMethod("putObject", Object.class, long.class, Object.class);
        objectFieldOffset = allocator.getMethod("objectFieldOffset", Field.class);
        CHEST_MODEL = new ChestModel(ChestModel.createSingleBodyLayer().bakeRoot());
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                String result;
                try {
                    result = run(c);
                } catch (Throwable t) {
                    result = "error " + t + "\n";
                }
                out.print("case " + c.get("name").getAsString() + "\n" + result);
            }
        }
    }
}
