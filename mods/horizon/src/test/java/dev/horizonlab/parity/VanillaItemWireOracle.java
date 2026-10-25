package dev.horizonlab.parity;

import io.netty.buffer.Unpooled;
import it.unimi.dsi.fastutil.objects.Reference2ObjectMap;
import it.unimi.dsi.fastutil.objects.Reference2ObjectMaps;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.Optional;
import net.minecraft.core.Holder;
import net.minecraft.core.component.DataComponentPatch;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.component.TypedDataComponent;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.world.entity.decoration.painting.PaintingVariant;
import net.minecraft.world.item.JukeboxPlayable;
import net.minecraft.world.item.JukeboxSong;
import net.minecraft.world.item.component.MapPostProcessing;
import net.minecraft.util.Unit;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;

/**
 * Item stacks as a server sends them (ItemStack.OPTIONAL_STREAM_CODEC), for
 * every data component: each default component value of every vanilla item
 * set on a stick (so it is in the patch), the SNBT fixtures given in a file
 * (one `label<TAB>stack SNBT` per line), and a few patches built directly.
 * Prints one JSON line per stack: its label, the encoded bytes in hex and the
 * patch's component type ids in encoding order (added, then removed), or
 * the error.
 */
public final class VanillaItemWireOracle {
    private static final int PER_TYPE = 12;

    public static void main(String[] args) throws Exception {
        ParityData.init();
        List<String[]> fixtures = new ArrayList<>();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank() || line.startsWith("#")) continue;
            fixtures.add(line.split("\t", 2));
        }
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))) {
            // Each distinct default value, once per component type and
            // encoding, and at most PER_TYPE of them for one type.
            Set<String> seen = new HashSet<>();
            Map<DataComponentType<?>, Integer> perType = new HashMap<>();
            for (var item : BuiltInRegistries.ITEM) {
                for (TypedDataComponent<?> c : item.components()) {
                    ItemStack stack = new ItemStack(Items.STICK);
                    if (!set(stack, c)) continue;
                    String hex = encode(stack);
                    if (hex == null || perType.getOrDefault(c.type(), 0) >= PER_TYPE) continue;
                    if (!seen.add(BuiltInRegistries.DATA_COMPONENT_TYPE.getKey(c.type()) + hex)) continue;
                    perType.merge(c.type(), 1, Integer::sum);
                    print(out, "default " + BuiltInRegistries.ITEM.getKey(item) + " " + BuiltInRegistries.DATA_COMPONENT_TYPE.getKey(c.type()), stack);
                }
            }
            for (String[] f : fixtures) {
                try {
                    print(out, f[0], ParityData.stack(f[1]));
                } catch (Exception e) {
                    out.println("{\"label\":" + json(f[0]) + ",\"error\":" + json(String.valueOf(e)) + "}");
                }
            }
            // Components without a persistent codec, direct holders item SNBT
            // can't name, and removals.
            ItemStack direct = new ItemStack(Items.STICK);
            direct.set(DataComponents.ADDITIONAL_TRADE_COST, 4);
            direct.set(DataComponents.MAP_POST_PROCESSING, MapPostProcessing.SCALE);
            direct.set(DataComponents.JUKEBOX_PLAYABLE, new JukeboxPlayable(Holder.direct(
                new JukeboxSong(SoundEvents.ARMOR_EQUIP_CHAIN, Component.literal("S"), 10f, 3))));
            direct.set(DataComponents.PAINTING_VARIANT, Holder.direct(new PaintingVariant(
                2, 1, Identifier.withDefaultNamespace("kebab"), Optional.of(Component.literal("T")), Optional.empty())));
            print(out, "non-persistent and direct", direct);
            ItemStack locked = new ItemStack(Items.STONE);
            locked.set(DataComponents.CREATIVE_SLOT_LOCK, Unit.INSTANCE);
            print(out, "creative_slot_lock", locked);
            ItemStack removed = new ItemStack(Items.APPLE, 3);
            removed.remove(DataComponents.FOOD);
            removed.remove(DataComponents.CONSUMABLE);
            removed.set(DataComponents.DAMAGE, 4);
            print(out, "removed food and consumable", removed);
            print(out, "empty", ItemStack.EMPTY);
        }
    }

    @SuppressWarnings("unchecked")
    private static <T> boolean set(ItemStack stack, TypedDataComponent<T> c) {
        if (stack.getPrototype().has(c.type()) && java.util.Objects.equals(stack.getPrototype().get(c.type()), c.value())) return false;
        stack.set((DataComponentType<T>) c.type(), c.value());
        return true;
    }

    private static String encode(ItemStack stack) {
        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), ParityData.access);
        try {
            ItemStack.OPTIONAL_STREAM_CODEC.encode(buf, stack);
        } catch (RuntimeException e) {
            return null;
        }
        byte[] bytes = new byte[buf.readableBytes()];
        buf.readBytes(bytes);
        return HexFormat.of().formatHex(bytes);
    }

    private static void print(PrintWriter out, String label, ItemStack stack) {
        String hex = encode(stack);
        if (hex == null) {
            out.println("{\"label\":" + json(label) + ",\"error\":\"encode failed\"}");
            return;
        }
        // DataComponentPatch's encoding order: its map's fast iteration, added
        // components, then removals.
        StringBuilder kinds = new StringBuilder();
        StringBuilder removed = new StringBuilder();
        for (var e : Reference2ObjectMaps.fastIterable(patchMap(stack.getComponentsPatch()))) {
            // Removed (package-private) marks a removal.
            StringBuilder to = e.getValue().getClass().getName().equals("net.minecraft.core.component.Removed") ? removed : kinds;
            to.append(to.isEmpty() ? "" : ",").append(id(e.getKey()));
        }
        out.println("{\"label\":" + json(label) + ",\"hex\":\"" + hex + "\",\"added\":[" + kinds + "],\"removed\":[" + removed + "]}");
    }

    @SuppressWarnings("unchecked")
    private static Reference2ObjectMap<DataComponentType<?>, Object> patchMap(DataComponentPatch patch) {
        try {
            Field map = DataComponentPatch.class.getDeclaredField("map");
            map.setAccessible(true);
            return (Reference2ObjectMap<DataComponentType<?>, Object>) map.get(patch);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }

    private static int id(DataComponentType<?> type) {
        return BuiltInRegistries.DATA_COMPONENT_TYPE.getId(type);
    }

    private static String json(String s) {
        return "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n") + "\"";
    }
}
