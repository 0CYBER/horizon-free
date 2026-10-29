package dev.horizonlab.parity;

import com.google.gson.*;
import io.netty.buffer.Unpooled;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.registries.Registries;
import net.minecraft.nbt.*;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.protocol.game.ServerboundRecipeBookChangeSettingsPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.stats.ServerRecipeBook;
import net.minecraft.world.entity.EntityEquipment;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.inventory.RecipeBookType;
import net.minecraft.world.item.crafting.Recipe;
import net.minecraft.world.item.crafting.RecipeHolder;
import net.minecraft.world.item.crafting.RecipeManager;

/**
 * Original oracle: the player's ServerRecipeBook under a sequence of
 * operations: awarding recipes (advancement rewards, /recipe give, crafting),
 * taking them (/recipe take), the client seeing a highlighted recipe, the
 * book settings, and saving and loading the recipeBook compound
 * (ServerRecipeBook.Packed.CODEC, loadUntrusted). After each step the count
 * the operation returned, the known and highlighted recipes (count and an
 * FNV-1a hash of the sorted ids) and, for a save, the compound are printed.
 */
public final class VanillaRecipeBookOracle {
    static long fnv(Collection<String> ids) {
        List<String> sorted = new ArrayList<>(ids);
        Collections.sort(sorted);
        long h = 0xcbf29ce484222325L;
        for (byte b : String.join(",", sorted).getBytes(StandardCharsets.UTF_8)) {
            h ^= (b & 0xff);
            h *= 0x100000001b3L;
        }
        return h;
    }

    static String ids(Collection<ResourceKey<Recipe<?>>> keys) {
        List<String> out = new ArrayList<>();
        for (var k : keys) out.add(k.identifier().toString());
        return out.size() + ":" + Long.toHexString(fnv(out));
    }

    static ResourceKey<Recipe<?>> key(String id) {
        return ResourceKey.create(Registries.RECIPE, Identifier.parse(id));
    }

    static String compound(CompoundTag tag) {
        List<String> keys = new ArrayList<>(tag.keySet());
        Collections.sort(keys);
        StringBuilder out = new StringBuilder();
        for (String k : keys) {
            Tag t = tag.get(k);
            out.append(' ').append(k).append('=');
            if (t instanceof ListTag list) {
                List<String> v = new ArrayList<>();
                for (Tag e : list) v.add(e.asString().orElse("?"));
                out.append(v.size()).append(':').append(Long.toHexString(fnv(v)));
            } else {
                out.append(t);
            }
        }
        return out.toString();
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var field = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        field.setAccessible(true);
        VanillaMenuOracle.unsafe = field.get(null);
        VanillaMenuOracle.allocate = VanillaMenuOracle.unsafe.getClass().getMethod("allocateInstance", Class.class);
        RecipeManager recipes = new RecipeManager(ParityData.lookup);
        recipes.finalizeRecipeLoading(FeatureFlags.VANILLA_SET);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                try {
                    var player = VanillaMenuOracle.make(VanillaMenuOracle.FakePlayer.class);
                    player.advancements = VanillaMenuOracle.make(net.minecraft.server.PlayerAdvancements.class);
                    VanillaMenuOracle.set(player.advancements, net.minecraft.server.PlayerAdvancements.class, "activeTriggers", new IdentityHashMap<>());
                    VanillaMenuOracle.set(player, ServerPlayer.class, "connection", VanillaMenuOracle.make(VanillaMenuOracle.Connection.class));
                    player.inv = new Inventory(player, new EntityEquipment());
                    ServerRecipeBook book = new ServerRecipeBook(recipes::listDisplaysForRecipe);
                    CompoundTag saved = null;
                    for (var opJson : f.getAsJsonArray("ops")) {
                        var op = opJson.getAsJsonObject();
                        String kind = op.get("op").getAsString();
                        int count = 0;
                        String extra = "";
                        switch (kind) {
                            case "give", "take" -> {
                                List<RecipeHolder<?>> holders = new ArrayList<>();
                                for (var id : op.getAsJsonArray("ids")) {
                                    recipes.byKey(key(id.getAsString())).ifPresent(holders::add);
                                }
                                count = kind.equals("give") ? book.addRecipes(holders, player) : book.removeRecipes(holders, player);
                            }
                            case "seen" -> book.removeHighlight(key(op.get("id").getAsString()));
                            case "set" -> {
                                var packet = new ServerboundRecipeBookChangeSettingsPacket(
                                    RecipeBookType.valueOf(op.get("type").getAsString()),
                                    op.get("open").getAsBoolean(), op.get("filtering").getAsBoolean());
                                var buf = new FriendlyByteBuf(Unpooled.buffer());
                                try {
                                    ServerboundRecipeBookChangeSettingsPacket.STREAM_CODEC.encode(buf, packet);
                                    byte[] wire = new byte[buf.readableBytes()];
                                    buf.getBytes(buf.readerIndex(), wire);
                                    extra = " packet=" + HexFormat.of().formatHex(wire);
                                    var decoded = ServerboundRecipeBookChangeSettingsPacket.STREAM_CODEC.decode(buf);
                                    book.setBookSetting(decoded.getBookType(), decoded.isOpen(), decoded.isFiltering());
                                } finally {
                                    buf.release();
                                }
                            }
                            case "save" -> {
                                saved = (CompoundTag) ServerRecipeBook.Packed.CODEC.encodeStart(NbtOps.INSTANCE, book.pack()).getOrThrow();
                                extra = compound(saved);
                            }
                            case "load" -> {
                                // The saved compound with ids the manager does not know; maybe a list missing.
                                CompoundTag tag = saved == null ? new CompoundTag() : saved.copy();
                                for (String list : List.of("recipes", "toBeDisplayed")) {
                                    if (tag.get(list) instanceof ListTag l) {
                                        l.add(StringTag.valueOf("minecraft:no_such_recipe"));
                                        l.add(StringTag.valueOf("stick"));
                                    }
                                }
                                if (op.has("drop")) tag.remove(op.get("drop").getAsString());
                                book = new ServerRecipeBook(recipes::listDisplaysForRecipe);
                                ServerRecipeBook fresh = book;
                                ServerRecipeBook.Packed.CODEC.parse(NbtOps.INSTANCE, tag).result()
                                    .ifPresent(p -> fresh.loadUntrusted(p, k -> recipes.byKey(k).isPresent()));
                            }
                            default -> throw new IllegalArgumentException(kind);
                        }
                        var packed = book.pack();
                        StringBuilder settings = new StringBuilder();
                        for (RecipeBookType t : RecipeBookType.values()) {
                            settings.append(book.isOpen(t) ? '1' : '0').append(book.isFiltering(t) ? '1' : '0');
                        }
                        out.println(kind + " " + count + " known=" + ids(packed.known()) + " highlight=" + ids(packed.highlight())
                            + " settings=" + settings + extra);
                    }
                    out.println("end");
                } catch (Throwable e) {
                    out.println("error-case " + e.getClass().getSimpleName() + " " + e.getMessage());
                    if (System.getenv("MENU_TRACE") != null) e.printStackTrace();
                }
            }
        }
    }
}
