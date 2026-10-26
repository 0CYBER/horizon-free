package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.nio.file.*;
import java.util.*;
import java.util.function.*;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.*;
import net.minecraft.util.Continuation;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.decoration.Cushion;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.blockscan.*;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;

public final class VanillaCushionPlacementOracle {
    record Cell(VoxelShape outline, VoxelShape collision, boolean suffocating) {}
    static final Cell AIR = new Cell(Shapes.empty(), Shapes.empty(), false);
    static final class FixtureBlock extends Block {
        FixtureBlock() { super(BlockBehaviour.Properties.of().dynamicShape()
            .setId(ResourceKey.create(Registries.BLOCK, Identifier.parse("horizon:cushion_fixture")))
            .isSuffocating((s, w, p) -> ((World)w).cell(p).suffocating())); }
        @Override protected VoxelShape getShape(BlockState s, BlockGetter w, BlockPos p, CollisionContext c) {
            return w instanceof World f ? f.cell(p).outline() : Shapes.empty();
        }
    }
    static BlockState state;
    static final class World extends ClientLevel {
        Map<BlockPos, Cell> cells;
        World() { super(null, null, null, null, 0, 0, null, false, 0, 0); }
        Cell cell(BlockPos p) { return cells.getOrDefault(p, AIR); }
        @Override public BlockState getBlockState(BlockPos p) { return state; }
        @Override public Iterable<VoxelShape> getBlockCollisions(Entity e, AABB b) {
            var result = new ArrayList<VoxelShape>();
            cells.forEach((p, c) -> result.add(c.collision().move(p)));
            return result;
        }
        @Override public BlockMatcher findBlocksIn(AABB bounds) {
            return new BlockMatcher(this) {
                @Override public boolean atLeastMatched(int n) { throw new AssertionError(); }
                @Override protected boolean anyMatched(Predicate<BlockState> p) { throw new AssertionError(); }
                @Override public void forEach(BiConsumer<BlockPos, BlockState> c) { throw new AssertionError(); }
                @Override public boolean forEachUntil(BlockStateConsumer consumer) {
                    for (var p : BlockPos.betweenClosed(bounds)) {
                        if (consumer.apply(p, state) == Continuation.ABORT) return true;
                    }
                    return false;
                }
            };
        }
    }
    static VoxelShape shape(JsonElement v) {
        var result = Shapes.empty();
        for (var e : v.getAsJsonArray()) {
            var b = e.getAsJsonArray();
            result = Shapes.or(result, Shapes.box(b.get(0).getAsDouble(), b.get(1).getAsDouble(), b.get(2).getAsDouble(), b.get(3).getAsDouble(), b.get(4).getAsDouble(), b.get(5).getAsDouble()));
        }
        return result;
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion(); net.minecraft.server.Bootstrap.bootStrap();
        // Supply one unregistered fixture block without changing the checks under test.
        var registry = net.minecraft.core.registries.BuiltInRegistries.BLOCK;
        var holders = net.minecraft.core.MappedRegistry.class.getDeclaredField("unregisteredIntrusiveHolders");
        holders.setAccessible(true); holders.set(registry, new IdentityHashMap<>());
        var frozen = net.minecraft.core.MappedRegistry.class.getDeclaredField("frozen");
        frozen.setAccessible(true); frozen.setBoolean(registry, false);
        state = new FixtureBlock().defaultBlockState();
        var u = Class.forName("sun.misc.Unsafe"); var f = u.getDeclaredField("theUnsafe"); f.setAccessible(true);
        var unsafe = f.get(null); var allocate = u.getMethod("allocateInstance", Class.class);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]))); var lines = Files.lines(Path.of(args[0]))) {
            for (var line : (Iterable<String>)lines::iterator) {
                var row = JsonParser.parseString(line).getAsJsonObject();
                var world = (World)allocate.invoke(unsafe, World.class); world.cells = new HashMap<>();
                for (var e : row.getAsJsonArray("cells")) {
                    var c = e.getAsJsonArray();
                    world.cells.put(new BlockPos(c.get(0).getAsInt(), c.get(1).getAsInt(), c.get(2).getAsInt()), new Cell(shape(c.get(3)), shape(c.get(4)), c.get(5).getAsBoolean()));
                }
                var p = row.getAsJsonArray("pos"); double x=p.get(0).getAsDouble(), y=p.get(1).getAsDouble(), z=p.get(2).getAsDouble();
                var bounds = new AABB(x-.5, y, z-.5, x+.5, y+.25, z+.5);
                out.println(Cushion.canBePlacedAt(world, bounds) + " " + Cushion.wouldSurviveAt(world, bounds));
            }
        }
    }
}
