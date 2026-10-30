package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.lang.reflect.Method;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.util.RandomSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;

/**
 * Native BlockItem placement-state selection (BlockPlaceContext resolution,
 * updatePlacementContext and getPlacementState) over generated neighbourhoods.
 * Entity obstruction can be scripted; nothing is written to the world.
 */
public final class VanillaBlockPlacementOracle {
    static Object unsafe;
    static Method allocate;
    static int errors;
    static final Set<BlockItem> bound = new HashSet<>();

    static <T> T make(Class<T> type) {
        try {
            return type.cast(allocate.invoke(unsafe, type));
        } catch (Exception e) {
            throw new AssertionError(e);
        }
    }

    static final class FakePlayer extends ServerPlayer {
        float yaw, pitch;
        boolean sneak;
        World world;
        ItemStack held;
        FakePlayer() {
            super(null, null, null, null);
        }
        @Override public float getViewXRot(final float a) { return pitch; }
        @Override public float getViewYRot(final float a) { return yaw; }
        @Override public float getXRot() { return pitch; }
        @Override public float getYRot() { return yaw; }
        @Override public Direction getDirection() { return Direction.fromYRot(yaw); }
        @Override public boolean isSecondaryUseActive() { return sneak; }
        @Override public boolean isShiftKeyDown() { return sneak; }
        @Override public boolean isDescending() { return sneak; }
        @Override public ItemStack getMainHandItem() { return held; }
        @Override public boolean canUseGameMasterBlocks() { return false; }
        @Override public net.minecraft.server.level.ServerLevel level() { return world; }
    }

    static final class World extends net.minecraft.server.level.ServerLevel {
        Map<BlockPos, BlockState> cells;
        RandomSource random;
        boolean obstructed;
        World() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }
        @Override public BlockState getBlockState(BlockPos p) { return cells.getOrDefault(p, net.minecraft.world.level.block.Blocks.AIR.defaultBlockState()); }
        @Override public FluidState getFluidState(BlockPos p) { return getBlockState(p).getFluidState(); }
        @Override public net.minecraft.world.level.block.entity.BlockEntity getBlockEntity(BlockPos p) { return null; }
        @Override public boolean isClientSide() { return false; }
        @Override public RandomSource getRandom() { return random; }
        @Override public int getMinY() { return -64; }
        @Override public int getHeight() { return 384; }
        @Override public boolean isUnobstructed(BlockState state, BlockPos pos, CollisionContext context) { return !obstructed; }
        @Override public boolean isUnobstructed(Entity entity) { return true; }
        @Override public void scheduleTick(BlockPos pos, Block block, int delay) { }
        @Override public net.minecraft.core.RegistryAccess registryAccess() { return RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY); }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return net.minecraft.world.flag.FeatureFlags.DEFAULT_FLAGS; }
        @Override public net.minecraft.world.level.border.WorldBorder getWorldBorder() { return new net.minecraft.world.level.border.WorldBorder(); }
        @Override public void gameEvent(net.minecraft.core.Holder<net.minecraft.world.level.gameevent.GameEvent> e, BlockPos p, net.minecraft.world.level.gameevent.GameEvent.Context c) { }
        @Override public int getSeaLevel() { return 63; }
        @Override public int getBrightness(net.minecraft.world.level.LightLayer layer, BlockPos p) { return layer == net.minecraft.world.level.LightLayer.SKY ? 15 : 0; }
        @Override public int getRawBrightness(BlockPos p, int darkening) { return 15 - darkening; }
        @Override public int getSkyDarken() { return 0; }
        @Override public List<Entity> getEntities(Entity except, net.minecraft.world.phys.AABB box, java.util.function.Predicate<? super Entity> filter) { return List.of(); }
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Path resources = Path.of(args[0]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(resources.resolve("block-tags.json"))).getAsJsonObject());
        VanillaLevelShapeOracle.bindFluidTags(resources.resolve("data/minecraft/tags/fluid"));
        var position = Entity.class.getDeclaredField("position");
        position.setAccessible(true);
        var allocator = Class.forName("sun.misc.Unsafe");
        var field = allocator.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        unsafe = field.get(null);
        allocate = allocator.getMethod("allocateInstance", Class.class);
        Method getPlacementState = BlockItem.class.getDeclaredMethod("getPlacementState", BlockPlaceContext.class);
        getPlacementState.setAccessible(true);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[2]))); var lines = Files.lines(Path.of(args[1]))) {
            for (var line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                var world = make(World.class);
                world.cells = new HashMap<>();
                for (var cell : f.getAsJsonArray("cells")) {
                    var c = cell.getAsJsonArray();
                    world.cells.put(new BlockPos(c.get(0).getAsInt(), c.get(1).getAsInt(), c.get(2).getAsInt()), Block.stateById(c.get(3).getAsInt()));
                }
                world.random = new LegacyRandomSource(f.get("seed").getAsLong());
                world.obstructed = f.has("obstructed") && f.get("obstructed").getAsBoolean();
                var player = make(FakePlayer.class);
                player.world = world;
                player.yaw = f.get("yaw").getAsFloat();
                player.pitch = f.get("pitch").getAsFloat();
                player.sneak = f.get("sneak").getAsBoolean();
                position.set(player, Vec3.ZERO);
                var item = (BlockItem) BuiltInRegistries.ITEM.getValue(Identifier.parse(f.get("item").getAsString()));
                if (bound.add(item)) {
                    item.builtInRegistryHolder().bindComponents(net.minecraft.core.component.DataComponentMap.EMPTY);
                }
                var h = f.getAsJsonArray("hit");
                var p = f.getAsJsonArray("pos");
                var hit = new BlockHitResult(new Vec3(h.get(0).getAsDouble(), h.get(1).getAsDouble(), h.get(2).getAsDouble()),
                    Direction.from3DDataValue(f.get("face").getAsInt()), new BlockPos(p.get(0).getAsInt(), p.get(1).getAsInt(), p.get(2).getAsInt()), false);
                String result;
                try {
                    player.held = new ItemStack(item);
                    var context = new BlockPlaceContext(player, InteractionHand.MAIN_HAND, player.held, hit);
                    if (!context.canPlace()) {
                        result = "none";
                    } else {
                        var updated = item.updatePlacementContext(context);
                        if (updated == null) {
                            result = "none";
                        } else {
                            var state = (BlockState) getPlacementState.invoke(item, updated);
                            var at = updated.getClickedPos();
                            result = state == null ? "none" : at.getX() + "," + at.getY() + "," + at.getZ() + " " + Block.getId(state);
                        }
                    }
                } catch (Exception | AssertionError e) {
                    Throwable cause = e instanceof java.lang.reflect.InvocationTargetException ite ? ite.getCause() : e;
                    result = "error " + cause.getClass().getSimpleName();
                    if (errors++ < 3) {
                        cause.printStackTrace();
                    }
                }
                out.println("place " + result);
            }
        }
    }
}
