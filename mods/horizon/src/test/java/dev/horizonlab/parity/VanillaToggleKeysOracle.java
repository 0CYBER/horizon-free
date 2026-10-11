package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.InputConstants;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.*;

/**
 * Drives the real Options key mappings (keyShift, keySprint, keyAttack and
 * keyUse are ToggleKeyMappings reading toggleCrouch/toggleSprint/toggleAttack/
 * toggleUse) through the real static KeyMapping.set/click, releaseAll,
 * restoreToggleStatesOnScreenClosed and resetToggleKeys, dispatched as
 * KeyboardHandler.keyPress, MouseHandler.onButton, Gui.setScreen and
 * LocalPlayer.respawn call them, and prints each mapping's down state and
 * pending clicks after every event.
 */
public final class VanillaToggleKeysOracle {
    static Field field(Class<?> type, String name) throws Exception {
        Field f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        var unsafe = field(u, "theUnsafe").get(null);
        Minecraft shell = (Minecraft) u.getMethod("allocateInstance", Class.class).invoke(unsafe, Minecraft.class);
        field(Minecraft.class, "fixerUpper").set(shell, net.minecraft.util.datafix.DataFixers.getDataFixer());
        field(Minecraft.class, "instance").set(null, shell);
        Options options = new Options(shell, Files.createTempDirectory("opts").toFile());
        KeyMapping[] mappings = {options.keyShift, options.keySprint, options.keyAttack, options.keyUse};
        List<OptionInstance<Boolean>> toggles = List.of(options.toggleCrouch(), options.toggleSprint(), options.toggleAttack(), options.toggleUse());
        Field keyField = field(KeyMapping.class, "key");
        Field clicks = field(KeyMapping.class, "clickCount");
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            int n = 0;
            for (var line : (Iterable<String>) lines::iterator) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                out.println("== " + n++);
                // A fresh session: everything up, no pending clicks or remembered releases.
                for (int i = 0; i < 4; i++) {
                    toggles.get(i).set(false);
                }
                KeyMapping.resetToggleKeys();
                KeyMapping.releaseAll();
                KeyMapping.restoreToggleStatesOnScreenClosed();
                for (var m : mappings) {
                    while (m.consumeClick()) {
                    }
                }
                for (var e : c.getAsJsonArray("ops")) {
                    var op = e.getAsJsonArray();
                    String name = op.get(0).getAsString();
                    switch (name) {
                        case "option" -> toggles.get(op.get(1).getAsInt()).set(op.get(2).getAsBoolean());
                        case "key" -> {
                            // KeyboardHandler.keyPress: action 0 releases; a press (1) or an SDL
                            // repeat (-1) sets and clicks only with no screen open.
                            var key = (InputConstants.Key) keyField.get(mappings[op.get(1).getAsInt()]);
                            int action = op.get(2).getAsInt();
                            boolean screen = op.get(3).getAsBoolean();
                            if (action == 0) {
                                KeyMapping.set(key, false);
                            } else if (!screen) {
                                KeyMapping.set(key, true);
                                KeyMapping.click(key);
                            }
                        }
                        case "button" -> {
                            // MouseHandler.onButton: only with no screen or overlay.
                            var key = (InputConstants.Key) keyField.get(mappings[op.get(1).getAsInt()]);
                            boolean pressed = op.get(2).getAsBoolean();
                            if (!op.get(3).getAsBoolean()) {
                                KeyMapping.set(key, pressed);
                                if (pressed) {
                                    KeyMapping.click(key);
                                }
                            }
                        }
                        case "open" -> KeyMapping.releaseAll();
                        case "close" -> KeyMapping.restoreToggleStatesOnScreenClosed();
                        case "respawn" -> KeyMapping.resetToggleKeys();
                        default -> throw new IllegalArgumentException(name);
                    }
                    var sb = new StringBuilder(name);
                    for (var m : mappings) {
                        sb.append(' ').append(m.isDown()).append(' ').append(clicks.getInt(m));
                    }
                    out.println(sb);
                }
            }
        }
        System.exit(0);
    }
}
