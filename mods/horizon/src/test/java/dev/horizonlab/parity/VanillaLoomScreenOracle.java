package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.client.gui.screens.inventory.CrafterScreen;
import net.minecraft.client.gui.screens.inventory.LoomScreen;
import net.minecraft.client.model.object.banner.BannerFlagModel;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.client.resources.model.sprite.SpriteId;
import net.minecraft.core.Holder;
import net.minecraft.core.NonNullList;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.contents.TranslatableContents;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.SimpleContainer;
import net.minecraft.world.inventory.*;
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.entity.BannerPattern;
import net.minecraft.world.level.block.entity.BannerPatternLayers;
import org.joml.Matrix3x2fStack;

/**
 * Original oracle for the loom and crafter screens' own drawing: vanilla's
 * LoomScreen.extractBackground (slot icons, scroller, pattern buttons with
 * their banner swatches and tooltips, the result banner or the six-layer
 * error) and CrafterScreen's background, disabled slots and redstone arrow,
 * on screens and menus whose fields come from the case, recording every
 * GuiGraphicsExtractor call they make.
 */
public final class VanillaLoomScreenOracle {
    static Object unsafe;
    static Method allocate;
    static JsonObject c;

    @SuppressWarnings("unchecked")
    static <T> T alloc(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        long offset = (long) unsafe.getClass().getMethod("objectFieldOffset", Field.class).invoke(unsafe, f);
        Class<?> t = f.getType();
        String setter = t == int.class ? "putInt" : t == boolean.class ? "putBoolean" : t == float.class ? "putFloat" : t == long.class ? "putLong" : "putObject";
        Class<?> param = t.isPrimitive() ? t : Object.class;
        unsafe.getClass().getMethod(setter, Object.class, long.class, param).invoke(unsafe, target, offset, value);
    }

    static net.minecraft.client.Minecraft mc;

    /** A stand-in client: Screen.extractBackground only asks its HUD for deferred subtitles (none). */
    static net.minecraft.client.Minecraft minecraft() throws Exception {
        if (mc == null) {
            mc = alloc(net.minecraft.client.Minecraft.class);
            var gui = alloc(net.minecraft.client.gui.Gui.class);
            put(gui, net.minecraft.client.gui.Gui.class, "hud", alloc(net.minecraft.client.gui.Hud.class));
            put(mc, net.minecraft.client.Minecraft.class, "gui", gui);
        }
        return mc;
    }

    static float round(float v) {
        return Math.round(v * 1e6f) / 1e6f;
    }

    /** Records the screens' GuiGraphicsExtractor calls. */
    public static final class Capture extends GuiGraphicsExtractor {
        JsonArray draws;
        Matrix3x2fStack stack;
        Map<TextureAtlasSprite, String> names = new IdentityHashMap<>();
        private Capture() { super(null, null, 0, 0); }
        @Override public int guiWidth() { return c.get("width").getAsInt(); }
        @Override public int guiHeight() { return c.get("height").getAsInt(); }
        @Override public Matrix3x2fStack pose() { return stack; }
        @Override public void nextStratum() { }
        @Override public void requestCursor(com.mojang.blaze3d.platform.cursor.CursorType type) { }
        JsonObject add(String type) {
            var o = new JsonObject();
            o.addProperty("t", type);
            draws.add(o);
            return o;
        }
        int tx(int x) { return x + Math.round(stack.m20()); }
        int ty(int y) { return y + Math.round(stack.m21()); }
        @Override public void fillGradient(int x0, int y0, int x1, int y1, int c1, int c2) {
            var o = add("gradient");
            o.addProperty("x1", x1); o.addProperty("y1", y1);
        }
        @Override public void fill(RenderPipeline p, int x0, int y0, int x1, int y1, int color) {
            var o = add("fill");
            o.addProperty("x0", tx(Math.min(x0, x1))); o.addProperty("y0", ty(Math.min(y0, y1)));
            o.addProperty("x1", tx(Math.max(x0, x1))); o.addProperty("y1", ty(Math.max(y0, y1)));
            o.addProperty("color", color);
        }
        @Override public void blitSprite(RenderPipeline p, Identifier id, int x, int y, int w, int h, int color) {
            var o = add("sprite");
            o.addProperty("id", id.getPath());
            o.addProperty("x", tx(x)); o.addProperty("y", ty(y)); o.addProperty("w", w); o.addProperty("h", h);
        }
        @Override public void blit(RenderPipeline p, Identifier texture, int x, int y, float u, float v, int w, int h, int srcW, int srcH, int texW, int texH, int color) {
            var o = add("blit");
            o.addProperty("texture", texture.getPath());
            o.addProperty("x", tx(x)); o.addProperty("y", ty(y)); o.addProperty("w", w); o.addProperty("h", h);
            o.addProperty("u", u); o.addProperty("v", v); o.addProperty("tw", texW); o.addProperty("th", texH);
        }
        @Override public void blit(Identifier location, int x0, int y0, int x1, int y1, float u0, float u1, float v0, float v1) {
            var o = add("atlas");
            o.addProperty("x0", tx(x0)); o.addProperty("y0", ty(y0)); o.addProperty("x1", tx(x1)); o.addProperty("y1", ty(y1));
            o.addProperty("u0", round(u0)); o.addProperty("u1", round(u1)); o.addProperty("v0", round(v0)); o.addProperty("v1", round(v1));
        }
        /** A stand-in sprite spanning the whole texture: the UVs a screen picks are then fractions of it. */
        @Override public TextureAtlasSprite getSprite(SpriteId id) {
            try {
                TextureAtlasSprite s = alloc(TextureAtlasSprite.class);
                put(s, TextureAtlasSprite.class, "atlasLocation", id.atlasLocation());
                put(s, TextureAtlasSprite.class, "u0", 0f);
                put(s, TextureAtlasSprite.class, "u1", 1f);
                put(s, TextureAtlasSprite.class, "v0", 0f);
                put(s, TextureAtlasSprite.class, "v1", 1f);
                var o = add("sprite_of");
                o.addProperty("id", id.texture().getPath());
                return s;
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        }
        @Override public void bannerPattern(BannerFlagModel flag, DyeColor base, BannerPatternLayers layers, int x0, int y0, int x1, int y1) {
            var o = add("banner");
            o.addProperty("base", base.getSerializedName());
            var a = new JsonArray();
            for (var layer : layers.layers()) {
                var l = new JsonArray();
                l.add(layer.pattern().unwrapKey().orElseThrow().identifier().toString());
                l.add(layer.color().getSerializedName());
                a.add(l);
            }
            o.add("layers", a);
            o.addProperty("x0", x0); o.addProperty("y0", y0); o.addProperty("x1", x1); o.addProperty("y1", y1);
        }
        @Override public void setTooltipForNextFrame(Component text, int x, int y) {
            var o = add("tooltip");
            o.addProperty("key", text.getContents() instanceof TranslatableContents t ? t.getKey() : text.getString());
        }
    }

    static Holder<BannerPattern> pattern(String id) {
        return ParityData.access.lookupOrThrow(Registries.BANNER_PATTERN).getOrThrow(ResourceKey.create(Registries.BANNER_PATTERN, Identifier.parse(id)));
    }

    static BannerPatternLayers layers(JsonArray list) {
        var b = new BannerPatternLayers.Builder();
        for (var l : list) {
            var p = l.getAsJsonArray();
            b.add(pattern(p.get(0).getAsString()), DyeColor.byName(p.get(1).getAsString(), DyeColor.WHITE));
        }
        return b.build();
    }

    static Slot slot(String item, int x, int y) {
        var container = new SimpleContainer(1);
        if (item != null) container.setItem(0, new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(item))));
        return new Slot(container, 0, x, y);
    }

    static String opt(String key) {
        return c.has(key) && !c.get(key).isJsonNull() ? c.get(key).getAsString() : null;
    }

    static void loom(Capture g) throws Exception {
        LoomMenu menu = alloc(LoomMenu.class);
        put(menu, LoomMenu.class, "bannerSlot", slot(opt("banner"), 13, 26));
        put(menu, LoomMenu.class, "dyeSlot", slot(opt("dye"), 33, 26));
        put(menu, LoomMenu.class, "patternSlot", slot(opt("pattern"), 23, 45));
        put(menu, LoomMenu.class, "resultSlot", slot(opt("result"), 143, 57));
        List<Holder<BannerPattern>> patterns = new ArrayList<>();
        for (var p : c.getAsJsonArray("patterns")) patterns.add(pattern(p.getAsString()));
        put(menu, LoomMenu.class, "selectablePatterns", patterns);
        DataSlot selected = DataSlot.standalone();
        selected.set(c.get("selected").getAsInt());
        put(menu, LoomMenu.class, "selectedBannerPatternIndex", selected);
        LoomScreen screen = alloc(LoomScreen.class);
        put(screen, net.minecraft.client.gui.screens.Screen.class, "minecraft", minecraft());
        put(screen, AbstractContainerScreen.class, "menu", menu);
        put(screen, AbstractContainerScreen.class, "imageWidth", 176);
        put(screen, AbstractContainerScreen.class, "imageHeight", 166);
        put(screen, AbstractContainerScreen.class, "leftPos", c.get("left").getAsInt());
        put(screen, AbstractContainerScreen.class, "topPos", c.get("top").getAsInt());
        put(screen, net.minecraft.client.gui.screens.Screen.class, "width", c.get("width").getAsInt());
        put(screen, net.minecraft.client.gui.screens.Screen.class, "height", c.get("height").getAsInt());
        put(screen, LoomScreen.class, "displayPatterns", c.get("display").getAsBoolean());
        put(screen, LoomScreen.class, "hasMaxPatterns", c.get("hasMax").getAsBoolean());
        put(screen, LoomScreen.class, "scrollOffs", c.get("scroll").getAsFloat());
        put(screen, LoomScreen.class, "startRow", c.get("startRow").getAsInt());
        put(screen, LoomScreen.class, "resultBannerPatterns", c.has("resultLayers") ? layers(c.getAsJsonArray("resultLayers")) : null);
        ItemStack dye = opt("dyeStack") == null ? ItemStack.EMPTY : new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(opt("dyeStack"))));
        put(screen, LoomScreen.class, "dyeStack", dye);
        try {
            screen.extractBackground(g, c.get("mouseX").getAsInt(), c.get("mouseY").getAsInt(), 0f);
        } catch (NullPointerException e) {
            // Lighting setup for the items after the background: no game renderer here.
            if (System.getenv("ORACLE_TRACE") != null) e.printStackTrace();
        }
    }

    static void crafter(Capture g) throws Exception {
        CrafterMenu menu = alloc(CrafterMenu.class);
        var data = new SimpleContainerData(10);
        var disabled = c.getAsJsonArray("disabled");
        for (int i = 0; i < 9; i++) data.set(i, disabled.get(i).getAsBoolean() ? 1 : 0);
        data.set(9, c.get("powered").getAsBoolean() ? 1 : 0);
        put(menu, CrafterMenu.class, "containerData", data);
        CrafterScreen screen = alloc(CrafterScreen.class);
        put(screen, net.minecraft.client.gui.screens.Screen.class, "minecraft", minecraft());
        put(screen, AbstractContainerScreen.class, "menu", menu);
        put(screen, AbstractContainerScreen.class, "imageWidth", 176);
        put(screen, AbstractContainerScreen.class, "imageHeight", 166);
        int left = c.get("left").getAsInt(), top = c.get("top").getAsInt();
        put(screen, AbstractContainerScreen.class, "leftPos", left);
        put(screen, AbstractContainerScreen.class, "topPos", top);
        put(screen, net.minecraft.client.gui.screens.Screen.class, "width", c.get("width").getAsInt());
        put(screen, net.minecraft.client.gui.screens.Screen.class, "height", c.get("height").getAsInt());
        screen.extractBackground(g, 0, 0, 0f);
        // AbstractContainerScreen.extractSlots: the slots under the menu's offset.
        g.stack.pushMatrix();
        g.stack.translate(left, top);
        Method extractSlot = CrafterScreen.class.getDeclaredMethod("extractSlot", GuiGraphicsExtractor.class, Slot.class, int.class, int.class);
        extractSlot.setAccessible(true);
        for (int y = 0; y < 3; y++) {
            for (int x = 0; x < 3; x++) {
                int i = x + y * 3;
                if (!disabled.get(i).getAsBoolean()) continue;
                var slot = new CrafterSlot(new SimpleContainer(9), i, 26 + x * 18, 17 + y * 18, menu);
                put(slot, Slot.class, "index", i);
                extractSlot.invoke(screen, g, slot, -100, -100);
            }
        }
        g.stack.popMatrix();
        Method redstone = CrafterScreen.class.getDeclaredMethod("extractRedstone", GuiGraphicsExtractor.class);
        redstone.setAccessible(true);
        redstone.invoke(screen, g);
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var field = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        field.setAccessible(true);
        unsafe = field.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        var out = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank()) continue;
            c = JsonParser.parseString(line).getAsJsonObject();
            var result = new JsonObject();
            try {
                var g = alloc(Capture.class);
                g.draws = new JsonArray();
                g.stack = new Matrix3x2fStack(16);
                put(g, GuiGraphicsExtractor.class, "pose", g.stack);
                if (c.get("screen").getAsString().equals("loom")) loom(g); else crafter(g);
                result.add("draws", g.draws);
            } catch (Throwable e) {
                Throwable t = e instanceof InvocationTargetException ite ? ite.getCause() : e;
                result.addProperty("error", t.toString());
                if (System.getenv("ORACLE_TRACE") != null) t.printStackTrace();
            }
            out.append(result).append('\n');
        }
        Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
    }
}
