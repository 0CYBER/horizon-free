package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.function.Predicate;
import java.util.stream.Stream;
import net.minecraft.ChatFormatting;
import net.minecraft.client.ClientRecipeBook;
import net.minecraft.client.searchtree.FullTextSearchTree;
import net.minecraft.client.searchtree.SearchTree;
import net.minecraft.core.Registry;
import net.minecraft.util.context.ContextMap;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.TooltipFlag;
import net.minecraft.world.item.crafting.display.SlotDisplayContext;
import net.minecraft.client.gui.screens.recipebook.RecipeCollection;
import net.minecraft.client.gui.screens.recipebook.SearchRecipeBookCategory;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.entity.player.StackedItemContents;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.crafting.ExtendedRecipeBookCategory;
import net.minecraft.world.item.crafting.RecipeBookCategories;
import net.minecraft.world.item.crafting.RecipeManager;
import net.minecraft.world.item.crafting.display.FurnaceRecipeDisplay;
import net.minecraft.world.item.crafting.display.RecipeDisplay;
import net.minecraft.world.item.crafting.display.RecipeDisplayEntry;
import net.minecraft.world.item.crafting.display.ShapedCraftingRecipeDisplay;
import net.minecraft.world.item.crafting.display.ShapelessCraftingRecipeDisplay;

/**
 * Original oracle for what the recipe book screen lists: the client's
 * ClientRecipeBook receives the displays of the known recipes (in the order
 * the server sent them, then later unlocks and removals) and rebuilds its
 * collections; each collection selects its recipes for the open menu
 * (CraftingRecipeBookComponent / FurnaceRecipeBookComponent.selectMatchingRecipes)
 * with the stacked contents of the inventory and the menu slots; then for
 * every tab of the book RecipeBookTabButton.updateVisibility and
 * RecipeBookComponent.updateCollections (without a search) decide the
 * buttons. Printed per tab: visibility, the page count, and each button's
 * displays with their selected and craftable flags. A case may carry a search
 * text: the collections are searched as SessionSearchTrees.updateRecipes builds
 * the tree (a FullTextSearchTree over the tooltip lines and ids of the results)
 * and RecipeBookComponent.updateCollections applies it.
 */
public final class VanillaRecipeBookViewOracle {
    record Tab(ExtendedRecipeBookCategory category, String name) {}

    static List<Tab> tabs(String type) {
        return switch (type) {
            case "crafting" -> List.of(new Tab(SearchRecipeBookCategory.CRAFTING, "search"),
                new Tab(RecipeBookCategories.CRAFTING_EQUIPMENT, "crafting_equipment"),
                new Tab(RecipeBookCategories.CRAFTING_BUILDING_BLOCKS, "crafting_building_blocks"),
                new Tab(RecipeBookCategories.CRAFTING_MISC, "crafting_misc"),
                new Tab(RecipeBookCategories.CRAFTING_REDSTONE, "crafting_redstone"));
            case "furnace" -> List.of(new Tab(SearchRecipeBookCategory.FURNACE, "search"),
                new Tab(RecipeBookCategories.FURNACE_FOOD, "furnace_food"),
                new Tab(RecipeBookCategories.FURNACE_BLOCKS, "furnace_blocks"),
                new Tab(RecipeBookCategories.FURNACE_MISC, "furnace_misc"));
            case "blast_furnace" -> List.of(new Tab(SearchRecipeBookCategory.BLAST_FURNACE, "search"),
                new Tab(RecipeBookCategories.BLAST_FURNACE_BLOCKS, "blast_furnace_blocks"),
                new Tab(RecipeBookCategories.BLAST_FURNACE_MISC, "blast_furnace_misc"));
            default -> List.of(new Tab(SearchRecipeBookCategory.SMOKER, "search"),
                new Tab(RecipeBookCategories.SMOKER_FOOD, "smoker_food"));
        };
    }

    static List<RecipeDisplayEntry> displays(RecipeManager recipes, String id) {
        List<RecipeDisplayEntry> out = new ArrayList<>();
        recipes.listDisplaysForRecipe(ResourceKey.create(Registries.RECIPE, Identifier.parse(id)), out::add);
        return out;
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        RecipeManager recipes = new RecipeManager(ParityData.lookup);
        recipes.finalizeRecipeLoading(FeatureFlags.VANILLA_SET);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                try {
                    ClientRecipeBook book = new ClientRecipeBook();
                    // ClientPacketListener.handleRecipeBookAdd / handleRecipeBookRemove.
                    for (var batch : f.getAsJsonArray("history")) {
                        var b = batch.getAsJsonObject();
                        String op = b.get("op").getAsString();
                        if (op.equals("replace")) book.clear();
                        for (var id : b.getAsJsonArray("ids")) {
                            for (RecipeDisplayEntry entry : displays(recipes, id.getAsString())) {
                                if (op.equals("remove")) book.remove(entry.id());
                                else book.add(entry);
                            }
                        }
                        book.rebuildCollections();
                    }
                    StackedItemContents contents = new StackedItemContents();
                    for (var s : f.getAsJsonArray("inventory")) contents.accountSimpleStack(ParityData.stack(s.getAsString()));
                    String type = f.get("type").getAsString();
                    int grid = f.get("grid").getAsInt();
                    for (var s : f.getAsJsonArray("slots")) {
                        ItemStack stack = ParityData.stack(s.getAsString());
                        // A furnace's SimpleContainer counts every slot; the crafting grid only simple stacks.
                        if (type.equals("crafting")) contents.accountSimpleStack(stack);
                        else contents.accountStack(stack);
                    }
                    Predicate<RecipeDisplay> canDisplay = type.equals("crafting")
                        ? display -> switch (display) {
                            case ShapedCraftingRecipeDisplay shaped -> grid >= shaped.width() && grid >= shaped.height();
                            case ShapelessCraftingRecipeDisplay shapeless -> grid * grid >= shapeless.ingredients().size();
                            default -> false;
                        }
                        : display -> display instanceof FurnaceRecipeDisplay;
                    List<Tab> tabs = tabs(type);
                    for (Tab tab : tabs) {
                        for (RecipeCollection c : book.getCollection(tab.category())) c.selectRecipes(contents, canDisplay);
                    }
                    boolean filtering = f.get("filtering").getAsBoolean();
                    String query = f.has("search") ? f.get("search").getAsString() : "";
                    Set<RecipeCollection> found = null;
                    if (!query.isEmpty()) {
                        // SessionSearchTrees.updateRecipes.
                        List<RecipeCollection> all = book.getCollections();
                        Registry<Item> itemRegistry = ParityData.access.lookupOrThrow(net.minecraft.core.registries.Registries.ITEM);
                        Item.TooltipContext tooltipContext = Item.TooltipContext.of(ParityData.access);
                        ContextMap recipeContext = SlotDisplayContext.fromLevel(ParityData.level);
                        SearchTree<RecipeCollection> tree = new FullTextSearchTree<>(
                            collection -> collection.getRecipes().stream().flatMap(e -> e.resultItems(recipeContext).stream())
                                .flatMap(item -> item.getTooltipLines(tooltipContext, null, TooltipFlag.Default.NORMAL).stream())
                                .map(l -> ChatFormatting.stripFormatting(l.getString()).trim()).filter(x -> !x.isEmpty()),
                            collection -> collection.getRecipes().stream().flatMap(e -> e.resultItems(recipeContext).stream())
                                .map(stack -> itemRegistry.getKey(stack.getItem())),
                            all);
                        if (System.getenv("DUMP_TOOLTIPS") != null) {
                            for (RecipeCollection c : all) for (var e : c.getRecipes()) for (var stack : e.resultItems(recipeContext)) {
                                System.err.println("TT " + e.id().index() + " " + itemRegistry.getKey(stack.getItem()) + " " + stack.getTooltipLines(tooltipContext, null, TooltipFlag.Default.NORMAL).stream().map(l -> l.getString()).toList());
                            }
                        }
                        found = new java.util.LinkedHashSet<>(tree.search(query.toLowerCase(Locale.ROOT)));
                    }
                    StringBuilder result = new StringBuilder();
                    for (Tab tab : tabs) {
                        List<RecipeCollection> collections = new ArrayList<>(book.getCollection(tab.category()));
                        boolean visible = tab.category() instanceof SearchRecipeBookCategory || collections.stream().anyMatch(RecipeCollection::hasAnySelected);
                        result.append(tab.name()).append(visible ? " shown" : " hidden");
                        collections.removeIf(c -> !c.hasAnySelected());
                        if (found != null) {
                            Set<RecipeCollection> set = found;
                            collections.removeIf(c -> !set.contains(c));
                        }
                        if (filtering) collections.removeIf(c -> !c.hasCraftable());
                        result.append(" pages=").append((int) Math.ceil(collections.size() / 20.0));
                        for (RecipeCollection c : collections) {
                            result.append(" [");
                            boolean first = true;
                            for (RecipeDisplayEntry e : c.getRecipes()) {
                                boolean selected = c.getSelectedRecipes(RecipeCollection.CraftableStatus.ANY).contains(e);
                                result.append(first ? "" : ",").append(e.id().index()).append(selected ? "s" : "").append(c.isCraftable(e.id()) ? "c" : "");
                                first = false;
                            }
                            result.append(']');
                        }
                        result.append(" | ");
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
