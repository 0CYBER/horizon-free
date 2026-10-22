package dev.horizonlab.parity;

import java.lang.reflect.*;
import java.nio.file.*;
import java.io.PrintWriter;
import net.minecraft.client.*;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ServerboundPlayerActionPacket;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.player.*;
import net.minecraft.world.item.*;
import net.minecraft.world.item.component.SwingAnimation;
import net.minecraft.world.level.GameType;

/** Runs the real key handler through its drop loop; stops before item-use handling. */
public final class VanillaHeldDropOracle {
    static Object unsafe;
    static Method allocate;
    static Field field(Class<?> type, String name) throws Exception {
        var f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }
    static <T> T make(Class<T> type) throws Exception {
        return type.cast(allocate.invoke(unsafe, type));
    }
    static final class Done extends RuntimeException {}
    static final class Client extends Minecraft {
        boolean all;
        Client() { super(null); }
        @Override public boolean hasControlDown() { return all; }
    }
    static final class Interface extends Gui {
        Interface() { super(null, null, null); }
        @Override public void handleKeybinds() {}
    }
    static final class Key extends KeyMapping {
        int clicks;
        Key() { super("", 0, null); }
        @Override public boolean consumeClick() { return clicks-- > 0; }
        @Override public boolean isDown() { return false; }
    }
    static final class Actor extends LocalPlayer {
        GameType mode;
        int swings;
        Actor() { super(null, null, null, null, null, null, false, null, null); }
        @Override public GameType gameMode() { return mode; }
        @Override public boolean isUsingItem() { throw new Done(); }
        @Override public boolean swing(InteractionHand hand, SwingAnimation animation, boolean sendPacket) {
            if (hand != InteractionHand.MAIN_HAND || !animation.equals(SwingAnimation.DEFAULT) || sendPacket)
                throw new AssertionError("unexpected drop swing");
            swings++;
            return true;
        }
    }
    static final class Connection extends ClientPacketListener {
        int drops;
        ServerboundPlayerActionPacket.Action expected;
        Connection() { super(null, null, null); }
        @Override public void send(Packet<?> packet) {
            if (packet instanceof ServerboundPlayerActionPacket action) {
                if (action.getAction() != expected) throw new AssertionError("unexpected drop action");
                drops++;
            }
        }
    }
    static final class Controller extends MultiPlayerGameMode {
        int dropped;
        Controller() { super(null, null); }
        // Run vanilla's actual prediction, packet dispatch and conditional swing.
        @Override public void dropItem(LocalPlayer player, boolean all) {
            int before = player.getInventory().getSelectedItem().getCount();
            super.dropItem(player, all);
            dropped += before - player.getInventory().getSelectedItem().getCount();
        }
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Items.AIR.builtInRegistryHolder().bindComponents(net.minecraft.core.component.DataComponentMap.EMPTY);
        Items.STONE.builtInRegistryHolder().bindComponents(net.minecraft.core.component.DataComponentMap.EMPTY);
        var u = Class.forName("sun.misc.Unsafe");
        unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        var handler = Minecraft.class.getDeclaredMethod("handleKeybinds");
        handler.setAccessible(true);
        var client = make(Client.class);
        var options = make(Options.class);
        for (var f : Options.class.getDeclaredFields()) {
            if (f.getType() == KeyMapping.class && !Modifier.isStatic(f.getModifiers())) {
                f.setAccessible(true);
                f.set(options, make(Key.class));
            }
        }
        var hotbar = new KeyMapping[9];
        for (int i = 0; i < 9; i++) hotbar[i] = make(Key.class);
        field(Options.class, "keyHotbarSlots").set(options, hotbar);
        field(Minecraft.class, "options").set(client, options);
        field(Minecraft.class, "gui").set(client, make(Interface.class));
        var actor = make(Actor.class);
        var inventory = new Inventory(actor, new PlayerEquipment(actor));
        field(Player.class, "inventory").set(actor, inventory);
        field(Minecraft.class, "player").set(client, actor);
        var controller = make(Controller.class);
        var connection = make(Connection.class);
        field(MultiPlayerGameMode.class, "minecraft").set(controller, client);
        field(MultiPlayerGameMode.class, "connection").set(controller, connection);
        field(Minecraft.class, "gameMode").set(client, controller);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (var mode : GameType.values()) for (boolean all : new boolean[]{false, true}) {
                for (int selected : new int[]{0, 8}) for (int count : new int[]{0, 1, 4, 64}) {
                    actor.mode = mode;
                    client.all = all;
                    inventory.clearContent();
                    inventory.setSelectedSlot(selected);
                    inventory.setItem(selected, new ItemStack(Items.STONE, count));
                    controller.dropped = 0;
                    actor.swings = 0;
                    connection.drops = 0;
                    connection.expected = all ? ServerboundPlayerActionPacket.Action.DROP_ALL_ITEMS
                        : ServerboundPlayerActionPacket.Action.DROP_ITEM;
                    // Two consecutive key presses also cover an emptied stack and repeated spectator drops.
                    for (int press = 0; press < 2; press++) {
                        ((Key) options.keyDrop).clicks = 1;
                        try {
                            handler.invoke(client);
                            throw new AssertionError("did not reach item-use boundary");
                        } catch (InvocationTargetException e) {
                            if (!(e.getCause() instanceof Done)) throw e;
                        }
                        out.println(inventory.getSelectedItem().getCount() + " " + controller.dropped
                            + " " + actor.swings + " " + connection.drops);
                    }
                }
            }
        }
    }
}
