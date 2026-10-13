package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.ClientRecipeBook;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.CycleButton;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.client.gui.components.WidgetSprites;
import net.minecraft.client.gui.components.WidgetTooltipHolder;
import net.minecraft.client.gui.screens.inventory.tooltip.ClientTooltipPositioner;
import net.minecraft.client.gui.screens.recipebook.*;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.chat.CommonComponents;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.util.Util;
import net.minecraft.util.context.ContextMap;
import net.minecraft.world.SimpleContainer;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.inventory.Slot;
import net.minecraft.world.inventory.tooltip.TooltipComponent;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.crafting.RecipeManager;
import net.minecraft.world.item.crafting.display.RecipeDisplayEntry;
import net.minecraft.world.item.crafting.display.RecipeDisplayId;
import net.minecraft.world.item.crafting.display.SlotDisplay;
import net.minecraft.world.item.crafting.display.SlotDisplayContext;
import net.minecraft.world.level.Level;
import org.joml.Matrix3x2fStack;

/**
 * Original oracle for the recipe book's drawing (RecipeBookComponent.extractRenderState,
 * its tooltips and GhostSlots.extractRenderState): the background, the real
 * search EditBox, RecipeBookTabButtons, the craftable filter CycleButton, a
 * RecipeBookPage of RecipeButtons over real RecipeCollections with its page
 * arrows and number, the OverlayRecipeComponent of a button and ghost slots,
 * each built as RecipeBookComponent.initVisuals and RecipeBookPage.init build
 * them, with the state the case gives (tab and button animations, the cycling
 * index, the search box, the mouse). Every GuiGraphicsExtractor call is recorded
 * with its pose. The hovered button's tooltip is recorded as its display stack
 * (RecipeButton.getTooltipText adds the item's own tooltip, compared elsewhere).
 */
public final class VanillaRecipeBookScreenOracle {
    static Object unsafe;
    static Method allocate;

    @SuppressWarnings("unchecked")
    static <T> T alloc(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static Field field(Class<?> owner, String name) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        long offset = (long) unsafe.getClass().getMethod("objectFieldOffset", Field.class).invoke(unsafe, f);
        Class<?> t = f.getType();
        String setter = t == int.class ? "putInt" : t == boolean.class ? "putBoolean" : t == float.class ? "putFloat" : t == long.class ? "putLong" : "putObject";
        Class<?> param = t.isPrimitive() ? t : Object.class;
        unsafe.getClass().getMethod(setter, Object.class, long.class, param).invoke(unsafe, target, offset, value);
    }

    static float round(float v) { return Math.round(v * 1e4f) / 1e4f; }

    /** A text's styled runs: [text, rgb or -1, italic]. */
    static JsonArray runs(FormattedCharSequence s) {
        var out = new JsonArray();
        StringBuilder text = new StringBuilder();
        int[] style = {Integer.MIN_VALUE, 0};
        s.accept((i, st, cp) -> {
            int color = st.getColor() == null ? -1 : st.getColor().getValue();
            int italic = st.isItalic() ? 1 : 0;
            if (text.length() > 0 && (color != style[0] || italic != style[1])) {
                var r = new JsonArray(); r.add(text.toString()); r.add(style[0]); r.add(style[1] == 1); out.add(r);
                text.setLength(0);
            }
            style[0] = color; style[1] = italic;
            text.appendCodePoint(cp);
            return true;
        });
        if (text.length() > 0) {
            var r = new JsonArray(); r.add(text.toString()); r.add(style[0]); r.add(style[1] == 1); out.add(r);
        }
        return out;
    }

    static String id(ItemStack s) { return s.isEmpty() ? "minecraft:air" : s.getItem().builtInRegistryHolder().key().identifier().toString(); }

    /** Records every GuiGraphicsExtractor call with the pose it was made under. */
    public static final class Capture extends GuiGraphicsExtractor {
        JsonArray draws = new JsonArray();
        Matrix3x2fStack stack = new Matrix3x2fStack(16);
        private Capture() { super(null, null, 0, 0); }
        @Override public Matrix3x2fStack pose() { return stack; }
        @Override public void nextStratum() { add("stratum"); }
        @Override public void requestCursor(com.mojang.blaze3d.platform.cursor.CursorType type) { }
        @Override public boolean containsPointInScissor(int x, int y) { return true; }
        JsonObject add(String type) {
            var o = new JsonObject();
            o.addProperty("t", type);
            var m = new JsonArray();
            for (float v : new float[] {stack.m00(), stack.m01(), stack.m10(), stack.m11(), stack.m20(), stack.m21()}) m.add(round(v));
            o.add("m", m);
            draws.add(o);
            return o;
        }
        @Override public void fill(RenderPipeline p, int x0, int y0, int x1, int y1, int color) {
            var o = add("fill");
            o.addProperty("x0", Math.min(x0, x1)); o.addProperty("y0", Math.min(y0, y1));
            o.addProperty("x1", Math.max(x0, x1)); o.addProperty("y1", Math.max(y0, y1));
            o.addProperty("color", color);
        }
        @Override public void textHighlight(int x0, int y0, int x1, int y1, boolean invert) {
            var o = add("highlight");
            o.addProperty("x0", x0); o.addProperty("y0", y0); o.addProperty("x1", x1); o.addProperty("y1", y1);
        }
        @Override public void blitSprite(RenderPipeline p, Identifier id, int x, int y, int w, int h, int color) {
            var o = add("sprite");
            o.addProperty("id", id.getPath());
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("w", w); o.addProperty("h", h);
        }
        @Override public void blit(RenderPipeline p, Identifier texture, int x, int y, float u, float v, int w, int h, int srcW, int srcH, int texW, int texH, int color) {
            var o = add("blit");
            o.addProperty("texture", texture.getPath());
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("w", w); o.addProperty("h", h);
            o.addProperty("u", u); o.addProperty("v", v); o.addProperty("tw", texW); o.addProperty("th", texH);
        }
        @Override public void text(Font font, FormattedCharSequence str, int x, int y, int color, boolean shadow) {
            var o = add("text");
            o.add("runs", runs(str));
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("color", color); o.addProperty("shadow", shadow);
        }
        @Override public void text(Font font, String str, int x, int y, int color, boolean shadow) {
            if (str != null) text(font, FormattedCharSequence.forward(str, net.minecraft.network.chat.Style.EMPTY), x, y, color, shadow);
        }
        void item(String kind, ItemStack s, int x, int y) {
            if (s.isEmpty()) return;
            var o = add(kind);
            o.addProperty("id", id(s)); o.addProperty("count", s.getCount());
            o.addProperty("x", x); o.addProperty("y", y);
        }
        @Override public void item(ItemStack s, int x, int y) { item("item", s, x, y); }
        @Override public void item(ItemStack s, int x, int y, int seed) { item("item", s, x, y); }
        @Override public void fakeItem(ItemStack s, int x, int y) { item("fake", s, x, y); }
        @Override public void fakeItem(ItemStack s, int x, int y, int seed) { item("fake", s, x, y); }
        @Override public void itemDecorations(Font font, ItemStack s, int x, int y, String count) { item("decorations", s, x, y); }
        @Override public void setTooltipForNextFrame(Font font, List<FormattedCharSequence> lines, Optional<TooltipComponent> image,
                                                     ClientTooltipPositioner positioner, int x, int y, boolean replace, Identifier style) {
            var o = add("tooltip");
            var a = new JsonArray();
            for (var l : lines) a.add(runs(l));
            o.add("lines", a);
        }
    }

    /** The window's text input area, which a focused EditBox moves: not tracked here. */
    static final class NoTextInput extends com.mojang.blaze3d.platform.TextInputManager {
        NoTextInput() { super(null); }
        @Override public void setTextInputArea(int x0, int y0, int x1, int y1) { }
    }

    static List<RecipeDisplayEntry> displays(RecipeManager recipes, String id) {
        List<RecipeDisplayEntry> out = new ArrayList<>();
        recipes.listDisplaysForRecipe(ResourceKey.create(Registries.RECIPE, Identifier.parse(id)), out::add);
        return out;
    }

    /** A collection of the given recipes' displays; `craftable` lists the recipes the inventory can craft. */
    @SuppressWarnings("unchecked")
    static RecipeCollection collection(RecipeManager recipes, JsonObject b) throws Exception {
        List<RecipeDisplayEntry> entries = new ArrayList<>();
        Set<RecipeDisplayId> craftable = (Set<RecipeDisplayId>) new HashSet<RecipeDisplayId>();
        for (var r : b.getAsJsonArray("recipes")) {
            var a = r.getAsJsonArray();
            for (var e : displays(recipes, a.get(0).getAsString())) {
                entries.add(e);
                if (a.get(1).getAsBoolean()) craftable.add(e.id());
            }
        }
        var c = new RecipeCollection(entries);
        ((Set<RecipeDisplayId>) field(RecipeCollection.class, "selected").get(c)).addAll(entries.stream().map(RecipeDisplayEntry::id).toList());
        ((Set<RecipeDisplayId>) field(RecipeCollection.class, "craftable").get(c)).addAll(craftable);
        return c;
    }

    /** The widget's tooltip shows on this frame when hovered (its delay has passed). */
    static void settled(Object widget) throws Exception {
        var holder = (WidgetTooltipHolder) field(net.minecraft.client.gui.components.AbstractWidget.class, "tooltip").get(widget);
        put(holder, WidgetTooltipHolder.class, "wasDisplayed", true);
        put(holder, WidgetTooltipHolder.class, "displayStartTime", Util.getMillis() - 1000L);
    }

    static JsonArray run(JsonObject c, Minecraft mc, RecipeManager recipes) throws Exception {
        var g = alloc(Capture.class);
        g.draws = new JsonArray();
        g.stack = new Matrix3x2fStack(16);
        String type = c.get("type").getAsString();
        boolean furnace = !type.equals("crafting");
        int width = c.get("width").getAsInt(), height = c.get("height").getAsInt();
        int xOffset = c.get("narrow").getAsBoolean() ? 0 : 86;
        int xo = (width - 147) / 2 - xOffset, yo = (height - 166) / 2;
        int mx = c.getAsJsonArray("mouse").get(0).getAsInt(), my = c.getAsJsonArray("mouse").get(1).getAsInt();
        int index = c.get("index").getAsInt();
        SlotSelectTime time = () -> index;
        boolean filtering = c.get("filtering").getAsBoolean();
        // RecipeBookComponent.extractRenderState.
        g.blit(RenderPipelines.GUI_TEXTURED, (Identifier) field(RecipeBookComponent.class, "RECIPE_BOOK_LOCATION").get(null), xo, yo, 1.0F, 1.0F, 147, 166, 256, 256);
        var s = c.getAsJsonObject("search");
        var search = new EditBox(mc.font, xo + 25, yo + 13, 81, 9 + 5, Component.translatable("itemGroup.search"));
        search.setMaxLength(50);
        search.setVisible(true);
        search.setTextColor(-1);
        search.setValue(s.get("value").getAsString());
        search.setHint((Component) field(RecipeBookComponent.class, "SEARCH_HINT").get(null));
        put(search, net.minecraft.client.gui.components.AbstractWidget.class, "focused", s.get("focused").getAsBoolean());
        put(search, EditBox.class, "displayPos", s.get("display").getAsInt());
        put(search, EditBox.class, "cursorPos", s.get("cursor").getAsInt());
        put(search, EditBox.class, "highlightPos", s.get("highlight").getAsInt());
        put(search, EditBox.class, "focusedTime", Util.getMillis() - s.get("blink").getAsLong());
        search.extractRenderState(g, mx, my, 1.0F);
        Class<?> tabOwner = switch (type) {
            case "crafting" -> CraftingRecipeBookComponent.class;
            case "furnace" -> net.minecraft.client.gui.screens.inventory.FurnaceScreen.class;
            case "blast_furnace" -> net.minecraft.client.gui.screens.inventory.BlastFurnaceScreen.class;
            default -> net.minecraft.client.gui.screens.inventory.SmokerScreen.class;
        };
        @SuppressWarnings("unchecked")
        var infos = (List<RecipeBookComponent.TabInfo>) field(tabOwner, "TABS").get(null);
        // updateTabs: the visible tabs in order.
        int n = 0;
        for (var t : c.getAsJsonArray("tabs")) {
            var tab = t.getAsJsonObject();
            var button = new RecipeBookTabButton(0, 0, infos.get(tab.get("tab").getAsInt()), b -> {});
            button.setPosition((width - 147) / 2 - xOffset - 30, (height - 166) / 2 + 3 + 27 * n++);
            if (tab.get("selected").getAsBoolean()) button.select();
            put(button, RecipeBookTabButton.class, "animationTime", tab.get("anim").getAsFloat());
            button.extractRenderState(g, mx, my, 1.0F);
        }
        RecipeBookComponent<?> component = furnace ? alloc(FurnaceRecipeBookComponent.class) : alloc(CraftingRecipeBookComponent.class);
        Method name = RecipeBookComponent.class.getDeclaredMethod("getRecipeFilterName");
        name.setAccessible(true);
        Method sprites = RecipeBookComponent.class.getDeclaredMethod("getFilterButtonTextures");
        sprites.setAccessible(true);
        if (furnace) put(component, FurnaceRecipeBookComponent.class, "recipeFilterName", Component.translatable(switch (type) {
            case "furnace" -> "gui.recipebook.toggleRecipes.smeltable";
            case "blast_furnace" -> "gui.recipebook.toggleRecipes.blastable";
            default -> "gui.recipebook.toggleRecipes.smokable";
        }));
        Component filterName = (Component) name.invoke(component);
        WidgetSprites filterSprites = (WidgetSprites) sprites.invoke(component);
        Component all = (Component) field(RecipeBookComponent.class, "ALL_RECIPES_TOOLTIP").get(null);
        var filter = CycleButton.booleanBuilder(filterName, all, filtering)
            .withTooltip(f -> f ? Tooltip.create(filterName) : Tooltip.create(all))
            .withSprite((b, f) -> filterSprites.get(f, b.isHoveredOrFocused()))
            .displayState(CycleButton.DisplayState.HIDE)
            .create(xo + 110, yo + 12, 26, 16, CommonComponents.EMPTY, (b, v) -> {});
        settled(filter);
        filter.extractRenderState(g, mx, my, 1.0F);
        // RecipeBookPage.init and updateCollections.
        var page = new RecipeBookPage(null, time, furnace);
        page.init(mc, xo, yo);
        List<RecipeCollection> collections = new ArrayList<>();
        for (var b : c.getAsJsonArray("collections")) collections.add(collection(recipes, b.getAsJsonObject()));
        put(page, RecipeBookPage.class, "currentPage", c.get("page").getAsInt());
        page.updateCollections(collections, false, filtering);
        @SuppressWarnings("unchecked")
        var buttons = (List<RecipeButton>) field(RecipeBookPage.class, "buttons").get(page);
        var anims = c.getAsJsonArray("buttonAnims");
        for (int i = 0; i < anims.size(); i++) put(buttons.get(i), RecipeButton.class, "animationTime", anims.get(i).getAsFloat());
        for (String arrow : new String[] {"forwardButton", "backButton"}) settled(field(RecipeBookPage.class, arrow).get(page));
        var overlay = (OverlayRecipeComponent) field(RecipeBookPage.class, "overlay").get(page);
        if (c.has("overlay")) {
            // RecipeBookPage.mouseClicked: right click on a button.
            var button = buttons.get(c.get("overlay").getAsInt());
            ContextMap context = SlotDisplayContext.fromLevel(mc.level);
            overlay.init(button.getCollection(), context, filtering, button.getX(), button.getY(), xo + 147 / 2, yo + 13 + 166 / 2, button.getWidth());
        }
        page.extractRenderState(g, xo, yo, mx, my, 1.0F);
        // RecipeBookPage.extractTooltip.
        var hovered = (RecipeButton) field(RecipeBookPage.class, "hoveredButton").get(page);
        if (hovered != null && !overlay.isVisible()) {
            var o = g.add("item_tooltip");
            o.addProperty("id", id(hovered.getDisplayStack()));
            o.addProperty("more", !hovered.isOnlyOption());
        }
        // GhostSlots.extractRenderState.
        if (c.has("ghost")) {
            var ghost = new GhostSlots(time);
            ContextMap context = SlotDisplayContext.fromLevel(mc.level);
            var container = new SimpleContainer(16);
            int i = 0;
            for (var e : c.getAsJsonArray("ghost")) {
                var gs = e.getAsJsonObject();
                var slot = new Slot(container, i++, gs.get("x").getAsInt(), gs.get("y").getAsInt());
                List<SlotDisplay> items = new ArrayList<>();
                for (var it : gs.getAsJsonArray("items")) items.add(new SlotDisplay.ItemStackSlotDisplay(new net.minecraft.world.item.ItemStackTemplate(net.minecraft.core.registries.BuiltInRegistries.ITEM.getValue(Identifier.parse(it.getAsString())))));
                SlotDisplay display = new SlotDisplay.Composite(items);
                Method set = GhostSlots.class.getDeclaredMethod(gs.get("result").getAsBoolean() ? "setResult" : "setInput", Slot.class, ContextMap.class, SlotDisplay.class);
                set.setAccessible(true);
                set.invoke(ghost, slot, context, display);
            }
            g.pose().pushMatrix();
            g.pose().translate(c.get("left").getAsInt(), c.get("top").getAsInt());
            ghost.extractRenderState(g, mc, c.get("big").getAsBoolean());
            g.pose().popMatrix();
        }
        return g.draws;
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        RecipeManager recipes = new RecipeManager(ParityData.lookup);
        recipes.finalizeRecipeLoading(FeatureFlags.VANILLA_SET);
        // A stand-in client: its font, a level for the slot display context, a player's book.
        Minecraft mc = alloc(Minecraft.class);
        put(mc, Minecraft.class, "font", ParityFont.create(ParityFont.resources()));
        var level = alloc(ClientLevel.class);
        put(level, Level.class, "registryAccess", ParityData.access);
        put(mc, Minecraft.class, "level", level);
        var player = alloc(LocalPlayer.class);
        put(player, LocalPlayer.class, "recipeBook", new ClientRecipeBook());
        put(mc, Minecraft.class, "player", player);
        put(mc, Minecraft.class, "lastInputType", net.minecraft.client.InputType.MOUSE);
        put(mc, Minecraft.class, "textInputManager", new NoTextInput());
        field(Minecraft.class, "instance").set(null, mc);
        var gson = new Gson();
        var out = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank()) continue;
            var c = JsonParser.parseString(line).getAsJsonObject();
            var result = new JsonObject();
            try {
                result.add("draws", run(c, mc, recipes));
            } catch (Throwable error) {
                var cause = error instanceof InvocationTargetException && error.getCause() != null ? error.getCause() : error;
                var sw = new java.io.StringWriter();
                cause.printStackTrace(new java.io.PrintWriter(sw));
                result.addProperty("error", sw.toString().lines().limit(8).reduce("", (a, b) -> a + b + " | "));
            }
            out.append(gson.toJson(result)).append('\n');
        }
        Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
    }
}
