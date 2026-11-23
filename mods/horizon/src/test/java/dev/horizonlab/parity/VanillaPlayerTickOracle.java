package dev.horizonlab.parity;

import com.mojang.authlib.GameProfile;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Locale;
import java.util.UUID;
import java.util.function.Predicate;
import net.minecraft.core.BlockPos;
import net.minecraft.core.RegistryAccess;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.Difficulty;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

/**
 * Runs the shipped `Player.tick` once per start position and prints where the player ends:
 * a spectator that flies, with the velocity each case gives it, so the tick's own move and
 * `Player.tick`'s world-edge clamp are both the shipped ones. The position starts where the
 * fixture puts it rather than from a move into it, and the seat a passenger is put on
 * (`Entity.rideTick`), the level of detail and the rest of the player's tick are outside
 * this oracle.
 */
public final class VanillaPlayerTickOracle {
    /** The level's block lookups reach for its server, which answers nothing else. */
    static MinecraftServer SERVER;

    /** A server level of air, at any position, with no entities to push the player. */
    static class Scene extends ServerLevel {
        ChunkAccess chunk;
        int next;
        /**
         * The border `LivingEntity`'s base tick reads for a player: the level's own default
         * (`WorldBorder.java:459`, 29,999,985 blocks from the centre). Six of the start
         * positions below stand past it, so their tick also takes the border's damage
         * (`LivingEntity.java:431-438`); damage moves nobody, so the position is still the
         * tick's own move and `Player.tick`'s clamp.
         */
        net.minecraft.world.level.border.WorldBorder border;

        /** An empty one: the moved player is no waypoint anyone tracks here. */
        net.minecraft.server.waypoints.ServerWaypointManager waypoints;

        @Override
        public net.minecraft.server.waypoints.ServerWaypointManager getWaypointManager() {
            return this.waypoints;
        }

        @Override
        public net.minecraft.world.level.border.WorldBorder getWorldBorder() {
            return this.border;
        }

        /** The fixture allocates past this, since the level's own set-up would start a server. */
        Scene() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }

        @Override
        public int getNextEntityId() {
            return ++this.next;
        }

        @Override
        public boolean isClientSide() {
            return false;
        }

        @Override
        public RegistryAccess registryAccess() {
            return ParityData.access;
        }

        /** The level's own set-up made this one; the fixture answers it from the data. */
        @Override
        public net.minecraft.world.damagesource.DamageSources damageSources() {
            return new net.minecraft.world.damagesource.DamageSources(ParityData.access);
        }

        @Override
        public MinecraftServer getServer() {
            return SERVER;
        }

        @Override
        public int getMinY() {
            return -64;
        }

        @Override
        public long getGameTime() {
            return 0L;
        }

        @Override
        public Difficulty getDifficulty() {
            return Difficulty.NORMAL;
        }

        @Override
        public BlockState getBlockState(BlockPos pos) {
            return Blocks.AIR.defaultBlockState();
        }

        @Override
        public FluidState getFluidState(BlockPos pos) {
            return Fluids.EMPTY.defaultFluidState();
        }

        @Override
        public boolean hasChunkAt(BlockPos pos) {
            return true;
        }

        @Override
        public boolean hasChunk(int x, int z) {
            return true;
        }

        @Override
        public ChunkAccess getChunk(int x, int z, ChunkStatus status, boolean create) {
            return this.chunk;
        }

        @Override
        public List<Entity> getEntities(Entity except, AABB bounds, Predicate<? super Entity> predicate) {
            return List.of();
        }

        @Override
        public void broadcastEntityEvent(Entity entity, byte event) {}

        @Override
        public void broadcastDamageEvent(Entity entity, net.minecraft.world.damagesource.DamageSource source) {}

        @Override
        public void gameEvent(net.minecraft.core.Holder<net.minecraft.world.level.gameevent.GameEvent> event,
                              Vec3 pos, net.minecraft.world.level.gameevent.GameEvent.Context context) {}

        @Override
        public void playSound(Entity except, double x, double y, double z, net.minecraft.sounds.SoundEvent sound,
                              net.minecraft.sounds.SoundSource source, float volume, float pitch) {}

        @Override
        public void playSound(Entity except, double x, double y, double z,
                              net.minecraft.core.Holder<net.minecraft.sounds.SoundEvent> sound,
                              net.minecraft.sounds.SoundSource source, float volume, float pitch) {}
    }

    /** The ticking spectator: the shipped `Player`, with no session behind it. */
    static class Actor extends Player {
        Actor(Scene scene) {
            super(scene, new GameProfile(UUID.fromString("01234567-89ab-cdef-0123-456789abcdef"), "Oracle"));
        }

        @Override
        public GameType gameMode() {
            return GameType.SPECTATOR;
        }

        /** No entities stand around to push the player: the fixture puts it where it is. */
        @Override
        protected void pushEntities() {}

        @Override
        public void checkBelowWorld() {}

        @Override
        protected void handlePortal() {}
    }

    /**
     * The fixture's start positions and the velocity each tick starts with, in the order
     * `player_world_edge_parity_tests` repeats: inside the edge, past it on each axis, on it,
     * at the clamp, above it, past it on both negative axes, and two ticks that cross it in
     * their own move.
     */
    static final Object[][] CASES = {
        {"inside_x", 29999980.0, 64.0, 8.0, 0.0, 0.0, 0.0},
        {"past_x", 30000020.0, 64.0, 8.0, 0.0, 0.0, 0.0},
        {"past_negative_x", -30000020.0, 64.0, -8.0, 0.0, 0.0, 0.0},
        {"past_z", 8.0, 64.0, 30000020.0, 0.0, 0.0, 0.0},
        {"past_negative_z", 8.0, 64.0, -30000020.0, 0.0, 0.0, 0.0},
        {"on_the_edge", 30000000.0, 64.0, 8.0, 0.0, 0.0, 0.0},
        {"at_the_clamp", 29999999.0, 64.0, 8.0, 0.0, 0.0, 0.0},
        {"high_y", 8.0, 30000000.0, 8.0, 0.0, 0.0, 0.0},
        {"past_negative_x_and_z", -30000020.0, 64.0, -30000020.0, 0.0, 0.0, 0.0},
        {"crossing_x", 29999980.0, 64.0, 8.0, 40.0, 0.0, 0.0},
        {"crossing_negative_z", 8.0, 64.0, -29999980.0, 0.0, 0.0, -40.0},
    };

    static String bits(double value) {
        return String.format(Locale.ROOT, "%016x", Double.doubleToRawLongBits(value));
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var runtime = Class.forName("sun.misc.Unsafe");
        var instance = runtime.getDeclaredField("theUnsafe");
        instance.setAccessible(true);
        var unsafe = instance.get(null);
        var allocate = runtime.getMethod("allocateInstance", Class.class);
        SERVER = (MinecraftServer) allocate.invoke(unsafe, net.minecraft.server.dedicated.DedicatedServer.class);
        var debug = MinecraftServer.class.getDeclaredField("debugSubscribers");
        debug.setAccessible(true);
        debug.set(SERVER, new net.minecraft.util.debug.ServerDebugSubscribers(SERVER));
        var scene = (Scene) allocate.invoke(unsafe, Scene.class);
        scene.border = new net.minecraft.world.level.border.WorldBorder();
        scene.waypoints = new net.minecraft.server.waypoints.ServerWaypointManager();
        var biome = net.minecraft.core.Holder.Reference.createStandAlone(
            new net.minecraft.core.HolderOwner<net.minecraft.world.level.biome.Biome>() {},
            net.minecraft.world.level.biome.Biomes.PLAINS);
        var ids = new net.minecraft.core.IdMapper<net.minecraft.core.Holder<net.minecraft.world.level.biome.Biome>>();
        ids.add(biome);
        var factory = new net.minecraft.world.level.chunk.PalettedContainerFactory(
            net.minecraft.world.level.chunk.Strategy.createForBlockStates(net.minecraft.world.level.block.Block.BLOCK_STATE_REGISTRY),
            Blocks.AIR.defaultBlockState(), null,
            net.minecraft.world.level.chunk.Strategy.createForBiomes(ids), biome, null);
        scene.chunk = new net.minecraft.world.level.chunk.ProtoChunk(
            new net.minecraft.world.level.ChunkPos(0, 0),
            net.minecraft.world.level.chunk.UpgradeData.EMPTY,
            net.minecraft.world.level.LevelHeightAccessor.create(-64, 384), factory, null);
        var out = new StringBuilder("[");
        for (var row : CASES) {
            var actor = new Actor(scene);
            actor.getAbilities().flying = true;
            actor.setPos((Double) row[1], (Double) row[2], (Double) row[3]);
            actor.setDeltaMovement((Double) row[4], (Double) row[5], (Double) row[6]);
            actor.tick();
            if (out.length() > 1) {
                out.append(',');
            }
            out.append("{\"case\":\"").append(row[0]).append('"')
                .append(",\"x\":\"").append(bits(actor.getX())).append('"')
                .append(",\"y\":\"").append(bits(actor.getY())).append('"')
                .append(",\"z\":\"").append(bits(actor.getZ())).append('"')
                .append('}');
        }
        out.append(']');
        Files.writeString(Path.of(args[0]), out + "\n");
        System.exit(0);
    }
}