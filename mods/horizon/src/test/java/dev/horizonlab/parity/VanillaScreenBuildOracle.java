package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.Window;
import com.mojang.text2speech.Narrator;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.GameNarrator;
import net.minecraft.client.StringSplitter;
import net.minecraft.client.gui.Font;
import net.minecraft.client.InputType;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.gui.Hud;
import net.minecraft.client.gui.components.AbstractButton;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.client.gui.components.ChatComponent;
import net.minecraft.client.gui.components.SpriteIconButton;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.narration.NarratableEntry;
import net.minecraft.client.gui.components.Renderable;
import net.minecraft.client.gui.screens.InBedChatScreen;
import net.minecraft.client.gui.screens.PauseScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.inventory.*;
import net.minecraft.client.gui.screens.recipebook.RecipeBookComponent;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.ClientRecipeContainer;
import net.minecraft.client.multiplayer.ClientSuggestionProvider;
import net.minecraft.client.multiplayer.chat.ChatAbilities;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.network.chat.Component;
import net.minecraft.world.SimpleContainer;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityEquipment;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.*;
import net.minecraft.world.entity.vehicle.minecart.MinecartCommandBlock;
import net.minecraft.world.level.block.entity.CommandBlockEntity;

/**
 * Original oracle for the container screens' built layout: the real
 * `Window.calculateScale` and GUI size of a window, then the real
 * `Screen.init(width, height)` of InventoryScreen, CraftingScreen,
 * FurnaceScreen, ContainerScreen (one to six rows), HopperScreen and
 * ShulkerBoxScreen at that size, with or without the recipe book offset.
 * Reports the panel (`leftPos`/`topPos`/`imageWidth`/`imageHeight`), the
 * slots' screen positions, and which slot `AbstractContainerScreen`'s hover
 * test answers for each cursor point the case names. For InBedChatScreen,
 * CommandBlockEditScreen and MinecartCommandBlockEditScreen it reports every
 * child widget's rectangle after `init` at another size and `resize` to the
 * case's.
 */
public final class VanillaScreenBuildOracle {
    static Object unsafe;
    static Method allocate;
    static Method hovered;
    static Minecraft mc;

    @SuppressWarnings("unchecked")
    static <T> T alloc(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        long offset = (long) unsafe.getClass().getMethod("objectFieldOffset", Field.class).invoke(unsafe, f);
        Class<?> t = f.getType();
        String setter = t == int.class ? "putInt" : t == boolean.class ? "putBoolean" : "putObject";
        Class<?> param = t.isPrimitive() ? t : Object.class;
        unsafe.getClass().getMethod(setter, Object.class, long.class, param).invoke(unsafe, target, offset, value);
    }

    static Object field(Object target, Class<?> owner, String name) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        return f.get(target);
    }

    /** A stand-in client: nothing narrates, the last input was the keyboard. */
    static Minecraft minecraft() throws Exception {
        if (mc != null) return mc;
        mc = alloc(Minecraft.class);
        put(mc, Minecraft.class, "lastInputType", InputType.KEYBOARD_TAB);
        var narrator = alloc(GameNarrator.class);
        var silent = (Narrator) Proxy.newProxyInstance(Narrator.class.getClassLoader(), new Class<?>[] {Narrator.class},
            (proxy, method, args) -> method.getReturnType() == boolean.class ? false : null);
        put(narrator, GameNarrator.class, "narrator", silent);
        put(mc, Minecraft.class, "narrator", narrator);
        // A furnace screen centers its title in init: any advance will do.
        var font = alloc(Font.class);
        put(font, Font.class, "splitter", new StringSplitter((codePoint, style) -> 6f));
        put(mc, Minecraft.class, "font", font);
        var instance = Minecraft.class.getDeclaredField("instance");
        instance.setAccessible(true);
        instance.set(null, mc);
        return mc;
    }

    /** A player with an inventory and a closed recipe book, enough for the screens' init. */
    static LocalPlayer player() throws Exception {
        var player = alloc(LocalPlayer.class);
        var inventory = new Inventory(player, new EntityEquipment());
        put(player, Player.class, "inventory", inventory);
        put(player, Player.class, "abilities", new Abilities());
        put(player, LocalPlayer.class, "recipeBook", new net.minecraft.client.ClientRecipeBook());
        // InventoryScreen takes the player's own menu, book component included.
        var menu = new InventoryMenu(inventory, true, player);
        put(player, Player.class, "inventoryMenu", menu);
        put(player, Player.class, "containerMenu", menu);
        // A furnace menu reads the player's level for its accepted inputs.
        var listener = alloc(ClientPacketListener.class);
        put(listener, ClientPacketListener.class, "recipes", new ClientRecipeContainer(Map.of(), null));
        var level = alloc(ClientLevel.class);
        put(level, ClientLevel.class, "connection", listener);
        put(player, Entity.class, "level", level);
        put(mc, Minecraft.class, "player", player);
        return player;
    }

    /** A pause screen on the client state `PauseScreen.init` reads. */
    static PauseScreen pauseScreen() throws Exception {
        var mc = minecraft();
        // The level adds the Options and World Options row; the local server
        // labels the disconnect button Return to Menu; the connection answers
        // the custom additions lookup (no dialog tag, no server links).
        put(mc, Minecraft.class, "level", alloc(ClientLevel.class));
        put(mc, Minecraft.class, "isLocalServer", true);
        // The friends button asks the social manager whether its list is on.
        put(mc, Minecraft.class, "playerSocialManager", alloc(net.minecraft.client.gui.screens.social.PlayerSocialManager.class));
        var player = player();
        var connection = alloc(ClientPacketListener.class);
        put(connection, ClientPacketListener.class, "registryAccess", ParityData.access);
        put(connection, net.minecraft.client.multiplayer.ClientCommonPacketListenerImpl.class, "serverLinks", net.minecraft.server.ServerLinks.EMPTY);
        put(player, LocalPlayer.class, "connection", connection);
        put(mc, Minecraft.class, "player", player);
        var screen = new PauseScreen(true);
        put(screen, Screen.class, "minecraft", mc);
        return screen;
    }

    /** A text box taking focus asks for no window's text input. */
    static final class NoTextInput extends com.mojang.blaze3d.platform.TextInputManager {
        NoTextInput() {
            super(null);
        }

        @Override
        public void onTextInputFocusChange(Object owner, boolean focused) {}
    }

    /**
     * Chat in bed, or a command block's or a minecart's edit screen, on a
     * player with no chat history, no restrictions and an empty command tree.
     */
    static Screen textScreen(String kind) throws Exception {
        var gui = alloc(Gui.class);
        var hud = alloc(Hud.class);
        var chat = alloc(ChatComponent.class);
        put(chat, ChatComponent.class, "recentChat", new net.minecraft.util.ArrayListDeque<String>(100));
        put(hud, Hud.class, "chat", chat);
        put(gui, Gui.class, "hud", hud);
        put(mc, Minecraft.class, "gui", gui);
        put(mc, Minecraft.class, "fontFilterFishy", mc.font);
        put(mc, Minecraft.class, "textInputManager", alloc(NoTextInput.class));
        put(mc.player, LocalPlayer.class, "chatAbilities", ChatAbilities.NO_RESTRICTIONS);
        var connection = alloc(ClientPacketListener.class);
        put(connection, ClientPacketListener.class, "commands", new com.mojang.brigadier.CommandDispatcher<ClientSuggestionProvider>());
        put(connection, ClientPacketListener.class, "suggestionsProvider", alloc(ClientSuggestionProvider.class));
        put(mc.player, LocalPlayer.class, "connection", connection);
        Screen screen = switch (kind) {
            case "chat" -> new InBedChatScreen("", false);
            case "command_block" -> {
                var entity = alloc(CommandBlockEntity.class);
                var base = Class.forName("net.minecraft.world.level.block.entity.CommandBlockEntity$1");
                put(entity, CommandBlockEntity.class, "commandBlock", alloc(base));
                yield new CommandBlockEditScreen(entity);
            }
            default -> {
                var cart = alloc(MinecartCommandBlock.class);
                var base = Class.forName("net.minecraft.world.entity.vehicle.minecart.MinecartCommandBlock$MinecartCommandBase");
                var block = alloc(base);
                put(block, net.minecraft.world.level.BaseCommandBlock.class, "command", "");
                put(cart, MinecartCommandBlock.class, "commandBlock", block);
                yield new MinecartCommandBlockEditScreen(cart);
            }
        };
        put(screen, Screen.class, "minecraft", mc);
        return screen;
    }

    static AbstractContainerScreen<?> screen(String kind, int rows, Inventory inventory, LocalPlayer player) throws Exception {
        return switch (kind) {
            case "inventory" -> new InventoryScreen(player);
            case "crafting" -> new CraftingScreen(new CraftingMenu(0, inventory, ContainerLevelAccess.NULL), inventory, Component.empty());
            case "furnace" -> new FurnaceScreen(new FurnaceMenu(0, inventory, new SimpleContainer(3), new SimpleContainerData(4)), inventory, Component.empty());
            case "hopper" -> new HopperScreen(new HopperMenu(0, inventory, new SimpleContainer(5)), inventory, Component.empty());
            case "shulker" -> new ShulkerBoxScreen(new ShulkerBoxMenu(0, inventory, new SimpleContainer(27)), inventory, Component.empty());
            case "chest" -> new ContainerScreen(chest(rows, inventory), inventory, Component.empty());
            default -> throw new IllegalArgumentException(kind);
        };
    }

    static ChestMenu chest(int rows, Inventory inventory) {
        return switch (rows) {
            case 1 -> ChestMenu.oneRow(0, inventory);
            case 2 -> ChestMenu.twoRows(0, inventory);
            case 3 -> ChestMenu.threeRows(0, inventory);
            case 4 -> ChestMenu.fourRows(0, inventory);
            case 5 -> ChestMenu.fiveRows(0, inventory);
            default -> ChestMenu.sixRows(0, inventory);
        };
    }

    static JsonArray pair(int x, int y) {
        var a = new JsonArray();
        a.add(x);
        a.add(y);
        return a;
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var field = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        field.setAccessible(true);
        unsafe = field.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        hovered = AbstractContainerScreen.class.getDeclaredMethod("getHoveredSlot", double.class, double.class);
        hovered.setAccessible(true);
        minecraft();
        var player = player();
        var inventory = player.getInventory();
        var out = new StringBuilder();
        var lines = Files.readAllLines(Path.of(args[0]));
        for (String line : lines) {
            if (line.isBlank()) continue;
            var c = JsonParser.parseString(line).getAsJsonObject();
            var result = new JsonObject();
            try {
                int fbWidth = c.getAsJsonArray("window").get(0).getAsInt();
                int fbHeight = c.getAsJsonArray("window").get(1).getAsInt();
                int requested = c.get("scale").getAsInt();
                // The real Window.calculateScale over the case's framebuffer.
                var window = alloc(Window.class);
                put(window, Window.class, "framebufferWidth", fbWidth);
                put(window, Window.class, "framebufferHeight", fbHeight);
                int scale = window.calculateScale(requested, false);
                window.setGuiScale(scale);
                int width = window.getGuiScaledWidth(), height = window.getGuiScaledHeight();
                var g = new JsonArray();
                g.add(scale);
                g.add(width);
                g.add(height);
                result.add("gui", g);

                String kind = c.get("kind").getAsString();
                if (kind.equals("chat") || kind.endsWith("command_block")) {
                    var screen = textScreen(kind);
                    // Opened at another size, then Minecraft.resizeGui's resize.
                    screen.init(width + 7, height + 3);
                    screen.resize(width, height);
                    var widgets = new JsonArray();
                    for (var child : screen.children()) {
                        if (!(child instanceof AbstractWidget widget)) continue;
                        var rect = pair(widget.getX(), widget.getY());
                        rect.add(widget.getWidth());
                        rect.add(widget.getHeight());
                        widgets.add(rect);
                    }
                    result.add("widgets", widgets);
                    out.append(result).append('\n');
                    continue;
                }
                if (kind.equals("pause")) {
                    var pause = pauseScreen();
                    pause.init(width, height);
                    var buttons = new JsonArray();
                    var icons = new JsonArray();
                    for (var child : pause.children()) {
                        if (!(child instanceof AbstractButton button)) continue;
                        if (button instanceof SpriteIconButton) {
                            icons.add(pair(button.getX(), button.getY()));
                        } else {
                            var rect = new JsonArray();
                            rect.add(button.getX());
                            rect.add(button.getY());
                            rect.add(button.getWidth());
                            rect.add(button.getMessage().getString());
                            buttons.add(rect);
                        }
                    }
                    var pauseJson = new JsonObject();
                    pauseJson.add("buttons", buttons);
                    pauseJson.add("icons", icons);
                    result.add("pause", pauseJson);
                    out.append(result).append('\n');
                    continue;
                }
                var screen = screen(c.get("kind").getAsString(), c.has("rows") ? c.get("rows").getAsInt() : 3, inventory, player);
                put(screen, Screen.class, "minecraft", mc);
                screen.init(width, height);
                if (c.has("book") && c.get("book").getAsBoolean() && screen instanceof AbstractRecipeBookScreen book) {
                    // The book's visibility moves the menu beside it (updateScreenPosition).
                    var component = (RecipeBookComponent) field(book, AbstractRecipeBookScreen.class, "recipeBookComponent");
                    put(component, RecipeBookComponent.class, "visible", true);
                    int imageWidth = (int) field(screen, AbstractContainerScreen.class, "imageWidth");
                    int moved = component.updateScreenPosition(width, imageWidth);
                    put(screen, AbstractContainerScreen.class, "leftPos", moved);
                }
                int left = (int) field(screen, AbstractContainerScreen.class, "leftPos");
                int top = (int) field(screen, AbstractContainerScreen.class, "topPos");
                var image = new JsonArray();
                image.add(left);
                image.add(top);
                image.add((int) field(screen, AbstractContainerScreen.class, "imageWidth"));
                image.add((int) field(screen, AbstractContainerScreen.class, "imageHeight"));
                result.add("image", image);
                result.add("origin", pair(left, top));
                var slots = new JsonArray();
                for (Slot slot : screen.getMenu().slots) {
                    slots.add(pair(left + slot.x, top + slot.y));
                }
                result.add("slots", slots);
                var hits = new JsonArray();
                for (var point : c.getAsJsonArray("points")) {
                    double x = point.getAsJsonArray().get(0).getAsDouble();
                    double y = point.getAsJsonArray().get(1).getAsDouble();
                    var hover = (Slot) hovered.invoke(screen, x, y);
                    if (hover == null) hits.add(JsonNull.INSTANCE);
                    else hits.add(pair(left + hover.x, top + hover.y));
                }
                result.add("hits", hits);
            } catch (Throwable e) {
                Throwable t = e instanceof InvocationTargetException ite ? ite.getCause() : e;
                result.addProperty("error", t.toString());
                if (System.getenv("ORACLE_TRACE") != null) t.printStackTrace();
            }
            out.append(result).append('\n');
        }
        Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
        System.exit(0);
    }
}
