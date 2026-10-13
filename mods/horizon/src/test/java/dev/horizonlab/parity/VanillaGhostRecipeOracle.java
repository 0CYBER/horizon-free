package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.gui.screens.recipebook.CraftingRecipeBookComponent;
import net.minecraft.client.gui.screens.recipebook.FurnaceRecipeBookComponent;
import net.minecraft.client.gui.screens.recipebook.GhostSlots;
import net.minecraft.client.gui.screens.recipebook.RecipeBookComponent;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.util.context.ContextMap;
import net.minecraft.world.entity.EntityEquipment;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.inventory.*;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.crafting.RecipeManager;
import net.minecraft.world.item.crafting.display.FurnaceRecipeDisplay;
import net.minecraft.world.item.crafting.display.RecipeDisplay;
import net.minecraft.world.item.crafting.display.RecipeDisplayEntry;
import net.minecraft.world.item.crafting.display.ShapedCraftingRecipeDisplay;
import net.minecraft.world.item.crafting.display.ShapelessCraftingRecipeDisplay;
import net.minecraft.world.item.crafting.display.SlotDisplayContext;

/**
 * Original oracle for the ghost recipe the recipe book fades into the menu
 * when a recipe is clicked: for every display of a recipe, the client's
 * CraftingRecipeBookComponent / FurnaceRecipeBookComponent.fillGhostRecipe
 * runs against the real inventory, crafting table or furnace menu (the
 * component is allocated without its constructor and given the menu) and
 * fills a GhostSlots. Printed per display, in the order the component set
 * them: the menu slot, whether it is the result, and the stacks the slot
 * cycles through (PlaceRecipeHelper's grid placement for shaped recipes, the
 * first slots for shapeless ones, the ingredient, the result and every fuel
 * for a furnace whose fuel slot is empty).
 */
public final class VanillaGhostRecipeOracle {
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var field = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        field.setAccessible(true);
        VanillaMenuOracle.unsafe = field.get(null);
        VanillaMenuOracle.allocate = VanillaMenuOracle.unsafe.getClass().getMethod("allocateInstance", Class.class);
        RecipeManager recipes = new RecipeManager(ParityData.lookup);
        recipes.finalizeRecipeLoading(FeatureFlags.VANILLA_SET);
        VanillaMenuOracle.recipes = recipes;
        ContextMap context = ContextMap.builder().set(SlotDisplayContext.REGISTRIES, ParityData.access).buildAndValidate(SlotDisplayContext.CONTEXT);
        Field ghostField = GhostSlots.class.getDeclaredField("ingredients");
        ghostField.setAccessible(true);
        Field menuField = RecipeBookComponent.class.getDeclaredField("menu");
        menuField.setAccessible(true);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                try {
                    var world = VanillaMenuOracle.make(VanillaMenuOracle.World.class);
                    world.server = VanillaMenuOracle.make(VanillaMenuOracle.Server.class);
                    var player = VanillaMenuOracle.make(VanillaMenuOracle.FakePlayer.class);
                    player.world = world;
                    var inv = new Inventory(player, new EntityEquipment());
                    player.inv = inv;
                    String kind = f.get("menu").getAsString();
                    AbstractContainerMenu menu = VanillaMenuOracle.menu(kind, player, inv);
                    VanillaMenuOracle.set(player, Player.class, "containerMenu", menu);
                    if (f.has("fuel") && !f.get("fuel").isJsonNull()) {
                        menu.getSlot(1).set(ParityData.stack(f.get("fuel").getAsString()));
                    }
                    RecipeBookComponent<?> component = kind.equals("inventory") || kind.equals("crafting")
                        ? VanillaMenuOracle.make(CraftingRecipeBookComponent.class)
                        : VanillaMenuOracle.make(FurnaceRecipeBookComponent.class);
                    menuField.set(component, menu);
                    Method fill = (component instanceof CraftingRecipeBookComponent ? CraftingRecipeBookComponent.class : FurnaceRecipeBookComponent.class)
                        .getDeclaredMethod("fillGhostRecipe", GhostSlots.class, RecipeDisplay.class, ContextMap.class);
                    fill.setAccessible(true);
                    int grid = kind.equals("inventory") ? 2 : 3;
                    List<RecipeDisplayEntry> displays = new ArrayList<>();
                    recipes.listDisplaysForRecipe(ResourceKey.create(Registries.RECIPE, Identifier.parse(f.get("recipe").getAsString())), displays::add);
                    StringBuilder result = new StringBuilder();
                    for (RecipeDisplayEntry entry : displays) {
                        RecipeDisplay display = entry.display();
                        // CraftingRecipeBookComponent.canDisplay: only what fits the grid is listed.
                        boolean fits = switch (display) {
                            case ShapedCraftingRecipeDisplay shaped -> grid >= shaped.width() && grid >= shaped.height();
                            case ShapelessCraftingRecipeDisplay shapeless -> grid * grid >= shapeless.ingredients().size();
                            case FurnaceRecipeDisplay furnace -> !kind.equals("inventory") && !kind.equals("crafting");
                            default -> false;
                        };
                        if (kind.equals("inventory") || kind.equals("crafting") ? display instanceof FurnaceRecipeDisplay : !(display instanceof FurnaceRecipeDisplay)) fits = false;
                        if (!fits) {
                            result.append("skip | ");
                            continue;
                        }
                        GhostSlots ghost = new GhostSlots(() -> 0);
                        fill.invoke(component, ghost, display, context);
                        var map = (Map<Slot, Object>) ghostField.get(ghost);
                        for (var e : map.entrySet()) {
                            Object slot = e.getValue();
                            Method items = slot.getClass().getDeclaredMethod("items");
                            Method isResult = slot.getClass().getDeclaredMethod("isResultSlot");
                            items.setAccessible(true);
                            isResult.setAccessible(true);
                            result.append(menu.slots.indexOf(e.getKey())).append((boolean) isResult.invoke(slot) ? "R" : "I").append('=');
                            boolean first = true;
                            for (ItemStack stack : (List<ItemStack>) items.invoke(slot)) {
                                result.append(first ? "" : ",").append(ParityData.canon(stack));
                                first = false;
                            }
                            result.append(' ');
                        }
                        result.append("| ");
                    }
                    out.println(result);
                } catch (Throwable e) {
                    out.println("error-case " + e.getClass().getSimpleName() + " " + e.getMessage());
                    if (System.getenv("MENU_TRACE") != null) e.printStackTrace();
                }
            }
        }
    }
}
