package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.EntityEquipment;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.entity.player.StackedContents;
import net.minecraft.world.entity.player.StackedItemContents;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.inventory.*;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.crafting.RecipeHolder;
import net.minecraft.world.item.crafting.RecipeManager;

/**
 * Original oracle: the server side of a recipe book click. For each case a
 * stand-in player (the menu oracle's) holds an inventory and opens the
 * inventory, crafting table or a furnace menu with some grid contents; then
 * RecipeBookMenu.handlePlacement (ServerPlaceRecipe) runs as
 * ServerGamePacketListenerImpl.handlePlaceRecipe calls it, and every menu
 * slot, the whole inventory, the drops and the post-place action are printed.
 *
 * The items available to the placement are kept in a hash map whose order
 * follows identity hash codes; the order this run used (a replica built the
 * same way iterates identically) goes to a second output for the Rust side.
 */
public final class VanillaPlaceRecipeOracle {
    @SuppressWarnings("unchecked")
    static List<String> order(Inventory inv, RecipeBookMenu menu) throws Exception {
        StackedItemContents contents = new StackedItemContents();
        inv.fillStackedContents(contents);
        menu.fillCraftSlotsStackedContents(contents);
        Field raw = StackedItemContents.class.getDeclaredField("raw");
        raw.setAccessible(true);
        StackedContents<Holder<Item>> stacked = (StackedContents<Holder<Item>>) raw.get(contents);
        List<String> out = new ArrayList<>();
        for (var entry : it.unimi.dsi.fastutil.objects.Reference2IntMaps.fastIterable(stacked.amounts)) {
            out.add(entry.getKey().unwrapKey().orElseThrow().identifier().toString());
        }
        return out;
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var field = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        field.setAccessible(true);
        VanillaMenuOracle.unsafe = field.get(null);
        VanillaMenuOracle.allocate = VanillaMenuOracle.unsafe.getClass().getMethod("allocateInstance", Class.class);
        RecipeManager recipes = new RecipeManager(ParityData.lookup);
        recipes.finalizeRecipeLoading(FeatureFlags.VANILLA_SET);
        VanillaMenuOracle.recipes = recipes;
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var orders = new PrintWriter(Files.newBufferedWriter(Path.of(args[2]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                List<String> order = List.of();
                try {
                    VanillaMenuOracle.drops.clear();
                    var world = VanillaMenuOracle.make(VanillaMenuOracle.World.class);
                    world.server = VanillaMenuOracle.make(VanillaMenuOracle.Server.class);
                    var player = VanillaMenuOracle.make(VanillaMenuOracle.FakePlayer.class);
                    player.world = world;
                    player.creative = f.get("creative").getAsBoolean();
                    player.advancements = VanillaMenuOracle.make(net.minecraft.server.PlayerAdvancements.class);
                    VanillaMenuOracle.set(player.advancements, net.minecraft.server.PlayerAdvancements.class, "activeTriggers", new IdentityHashMap<>());
                    VanillaMenuOracle.set(player, ServerPlayer.class, "connection", VanillaMenuOracle.make(VanillaMenuOracle.Connection.class));
                    var inv = new Inventory(player, new EntityEquipment());
                    player.inv = inv;
                    for (var pair : f.getAsJsonArray("inventory")) {
                        var p = pair.getAsJsonArray();
                        inv.setItem(p.get(0).getAsInt(), ParityData.stack(p.get(1).getAsString()));
                    }
                    inv.setSelectedSlot(f.get("selected").getAsInt());
                    AbstractContainerMenu menu = VanillaMenuOracle.menu(f.get("menu").getAsString(), player, inv);
                    VanillaMenuOracle.set(player, Player.class, "containerMenu", menu);
                    if (menu instanceof InventoryMenu im) VanillaMenuOracle.set(player, Player.class, "inventoryMenu", im);
                    for (var pair : f.getAsJsonArray("grid")) {
                        var p = pair.getAsJsonArray();
                        menu.getSlot(p.get(0).getAsInt()).set(ParityData.stack(p.get(1).getAsString()));
                    }
                    VanillaMenuOracle.drops.clear();
                    RecipeBookMenu book = (RecipeBookMenu) menu;
                    order = order(inv, book);
                    RecipeHolder<?> recipe = recipes.byKey(ResourceKey.create(Registries.RECIPE, Identifier.parse(f.get("recipe").getAsString()))).orElseThrow();
                    String post = "-";
                    // ServerGamePacketListenerImpl.handlePlaceRecipe
                    if (!recipe.value().placementInfo().isImpossibleToPlace()) {
                        post = book.handlePlacement(f.get("max").getAsBoolean(), player.isCreative(), recipe, world, inv).name();
                    }
                    StringBuilder line2 = new StringBuilder("post " + post + " | ");
                    line2.append(VanillaMenuOracle.state(menu, inv)).append(" | inv");
                    for (int i = 0; i < 41; i++) line2.append(' ').append(ParityData.canon(inv.getItem(i)));
                    out.println(line2);
                } catch (Throwable e) {
                    out.println("error-case " + e.getClass().getSimpleName());
                    if (System.getenv("MENU_TRACE") != null) e.printStackTrace();
                }
                orders.println(String.join(" ", order));
            }
        }
    }
}
