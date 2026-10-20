package dev.horizonlab.parity;

import com.google.gson.JsonParser;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.util.RandomSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;

/** Invoke shipped marker selection and ambient sampling in a dry barrier world. */
public final class VanillaMarkerParticlesOracle {
    static Field field(Class<?> type, String name) throws Exception {
        var field = type.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }
    public static final class Player extends LocalPlayer {
        ItemStack main, off;
        Player() { super(null, null, null, null, null, null, false, null, null); }
        @Override public ItemStack getMainHandItem() { return main; }
        @Override public ItemStack getItemInHand(InteractionHand hand) {
            return hand == InteractionHand.MAIN_HAND ? main : off;
        }
    }
    public static final class Mode extends MultiPlayerGameMode {
        GameType mode;
        Mode() { super(null, null); }
        @Override public GameType getPlayerMode() { return mode; }
    }
    public static final class MarkerLevel extends ClientLevel {
        java.io.BufferedWriter out;
        MarkerLevel() { super(null, null, null, null, 0, 0, null, false, 0L, 0); }
        @Override public BlockState getBlockState(BlockPos pos) { return Blocks.BARRIER.defaultBlockState(); }
        @Override public FluidState getFluidState(BlockPos pos) { return Fluids.EMPTY.defaultFluidState(); }
        @Override public void addParticle(ParticleOptions options, double x, double y, double z,
                                          double dx, double dy, double dz) {
            if (dx != 0 || dy != 0 || dz != 0) throw new AssertionError("marker velocity");
            try {
                out.write("marker " + x + " " + y + " " + z);
                out.newLine();
            } catch (java.io.IOException e) { throw new RuntimeException(e); }
        }
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        for (var item : BuiltInRegistries.ITEM) {
            item.builtInRegistryHolder().bindComponents(net.minecraft.core.component.DataComponentMap.EMPTY);
        }
        var allocator = Class.forName("sun.misc.Unsafe");
        var unsafe = field(allocator, "theUnsafe").get(null);
        var allocate = allocator.getMethod("allocateInstance", Class.class);
        var mc = (Minecraft) allocate.invoke(unsafe, Minecraft.class);
        var level = (MarkerLevel) allocate.invoke(unsafe, MarkerLevel.class);
        var player = (Player) allocate.invoke(unsafe, Player.class);
        var mode = (Mode) allocate.invoke(unsafe, Mode.class);
        field(ClientLevel.class, "minecraft").set(level, mc);
        mc.player = player;
        mc.gameMode = mode;
        var select = ClientLevel.class.getDeclaredMethod("getMarkerParticleTarget");
        select.setAccessible(true);
        try (var out = Files.newBufferedWriter(Path.of(args[1]))) {
            level.out = out;
            for (var line : Files.readAllLines(Path.of(args[0]))) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                mode.mode = GameType.byName(c.get("mode").getAsString());
                player.main = new ItemStack(BuiltInRegistries.ITEM.getValue(
                    Identifier.parse("minecraft:" + c.get("main").getAsString())), c.get("count").getAsInt());
                player.off = new ItemStack(BuiltInRegistries.ITEM.getValue(
                    Identifier.parse("minecraft:" + c.get("off").getAsString())));
                var block = (Block) select.invoke(level);
                out.write(block == null ? "none" : BuiltInRegistries.BLOCK.getKey(block).toString());
                out.newLine();
                var random = RandomSource.create(1057);
                field(Level.class, "random").set(level, random);
                level.animateTick(0, 64, 0);
                out.write("next " + random.nextInt());
                out.newLine();
            }
        }
    }
}
