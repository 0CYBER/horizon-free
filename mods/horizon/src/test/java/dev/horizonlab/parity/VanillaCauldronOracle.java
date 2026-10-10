package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.EntityEquipment;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.AbstractCauldronBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;

/**
 * Original oracle: a cauldron's useItemOn (CauldronInteractions) with a
 * recording stand-in level and player. Each case line is JSON: cauldron state
 * ("name|prop=value"), the held stack, creative, whether water is above. The
 * output line lists the result, block writes, sounds, the final inventory and
 * drops.
 */
public final class VanillaCauldronOracle {
    static Object unsafe;
    static java.lang.reflect.Method allocate;
    static final List<String> events = new ArrayList<>();
    static boolean underWater;

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) {
        try {
            return (T) allocate.invoke(unsafe, type);
        } catch (Exception e) {
            throw new AssertionError(e);
        }
    }

    static void set(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        f.set(target, value);
    }

    static String describe(BlockState state) {
        StringBuilder out = new StringBuilder(BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString()).append('|');
        List<Property<?>> props = new ArrayList<>(state.getProperties());
        props.sort(Comparator.comparing(Property::getName));
        boolean first = true;
        for (Property<?> p : props) {
            out.append(first ? "" : ",").append(p.getName()).append('=').append(name(state, p));
            first = false;
        }
        return out.toString();
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static String name(BlockState state, Property p) {
        return p.getName(state.getValue(p));
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static BlockState parse(String text) {
        String[] parts = text.split("\\|", -1);
        Block block = BuiltInRegistries.BLOCK.getValue(Identifier.parse(parts[0]));
        BlockState state = block.defaultBlockState();
        if (!parts[1].isEmpty()) {
            for (String kv : parts[1].split(",")) {
                String[] pair = kv.split("=");
                Property p = block.getStateDefinition().getProperty(pair[0]);
                state = state.setValue(p, (Comparable) p.getValue(pair[1]).orElseThrow());
            }
        }
        return state;
    }

    static final class World extends ServerLevel {
        World() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }
        @Override public boolean isClientSide() { return false; }
        @Override public boolean setBlock(BlockPos pos, BlockState state, int flags, int limit) { events.add("set " + describe(state)); return true; }
        @Override public FluidState getFluidState(BlockPos pos) { return underWater ? Fluids.WATER.defaultFluidState() : Fluids.EMPTY.defaultFluidState(); }
        @Override public void playSound(net.minecraft.world.entity.Entity p, double x, double y, double z, net.minecraft.sounds.SoundEvent s, net.minecraft.sounds.SoundSource c, float v, float pitch) {
            events.add("sound " + BuiltInRegistries.SOUND_EVENT.getKey(s));
        }
        @Override public void gameEvent(net.minecraft.core.Holder<net.minecraft.world.level.gameevent.GameEvent> e, net.minecraft.world.phys.Vec3 p, net.minecraft.world.level.gameevent.GameEvent.Context c) { }
        @Override public net.minecraft.core.RegistryAccess registryAccess() { return ParityData.access; }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return FeatureFlags.DEFAULT_FLAGS; }
    }

    static final class FakePlayer extends ServerPlayer {
        boolean creative;
        Inventory inv;
        FakePlayer() {
            super(null, null, null, null);
        }
        @Override public Inventory getInventory() { return inv; }
        @Override public boolean hasInfiniteMaterials() { return creative; }
        @Override public boolean isCreative() { return creative; }
        @Override public void setItemInHand(InteractionHand hand, ItemStack stack) { inv.setItem(0, stack); }
        @Override public ItemStack getItemInHand(InteractionHand hand) { return inv.getItem(0); }
        @Override public void awardStat(net.minecraft.stats.Stat<?> stat, int count) { }
        @Override public ItemEntity drop(ItemStack stack, boolean thrown, net.minecraft.util.Prediction p) {
            if (!stack.isEmpty()) events.add("drop " + ParityData.canon(stack));
            return null;
        }
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var field = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        field.setAccessible(true);
        unsafe = field.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                try {
                    events.clear();
                    underWater = f.get("under_water").getAsBoolean();
                    var world = make(World.class);
                    var player = make(FakePlayer.class);
                    player.creative = f.get("creative").getAsBoolean();
                    player.inv = new Inventory(player, new EntityEquipment());
                    for (var other : f.getAsJsonArray("inventory")) {
                        var pair = other.getAsJsonArray();
                        player.inv.setItem(pair.get(0).getAsInt(), ParityData.stack(pair.get(1).getAsString()));
                    }
                    ItemStack held = ParityData.stack(f.get("held").getAsString());
                    player.inv.setItem(0, held);
                    BlockState state = parse(f.get("state").getAsString());
                    var cauldron = (AbstractCauldronBlock) state.getBlock();
                    var use = net.minecraft.world.level.block.state.BlockBehaviour.class.getDeclaredMethod("useItemOn", ItemStack.class, BlockState.class,
                        net.minecraft.world.level.Level.class, BlockPos.class, Player.class, InteractionHand.class, net.minecraft.world.phys.BlockHitResult.class);
                    use.setAccessible(true);
                    InteractionResult result = (InteractionResult) use.invoke(cauldron, held, state, world, BlockPos.ZERO, player, InteractionHand.MAIN_HAND, null);
                    String kind = result == InteractionResult.SUCCESS ? "success" : result == InteractionResult.CONSUME ? "consume"
                        : result == InteractionResult.TRY_WITH_EMPTY_HAND ? "pass" : result.toString();
                    StringBuilder inventory = new StringBuilder();
                    for (int i = 0; i < 41; i++) {
                        ItemStack s = player.inv.getItem(i);
                        if (!s.isEmpty()) inventory.append(i).append('=').append(ParityData.canon(s)).append(' ');
                    }
                    out.println(kind + "\t" + String.join(";", events) + "\t" + inventory.toString().trim());
                } catch (Throwable e) {
                    out.println("error " + e.getClass().getSimpleName() + " " + e.getMessage());
                    if (System.getenv("CAULDRON_TRACE") != null) e.printStackTrace();
                }
            }
        }
    }
}
