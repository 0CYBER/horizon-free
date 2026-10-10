package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.function.*;
import net.minecraft.client.*;
import net.minecraft.util.datafix.DataFixers;

/**
 * Dumps Options.processOptions as a table: key, kind and default in processing
 * order. Used to (re)generate the Rust options table; the differential harness
 * then proves the Rust port agrees on loading and saving.
 */
public final class VanillaOptionsTableExporter {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        var uf = u.getDeclaredField("theUnsafe");
        uf.setAccessible(true);
        Object unsafe = uf.get(null);
        var allocate = u.getMethod("allocateInstance", Class.class);
        Minecraft shell = (Minecraft) allocate.invoke(unsafe, Minecraft.class);
        Field instance = Minecraft.class.getDeclaredField("instance");
        instance.setAccessible(true);
        instance.set(null, shell);
        Path work = Files.createTempDirectory("opts");
        Options options = new Options(shell, work.toFile());
        Method process = Options.class.getDeclaredMethod("processOptions", Class.forName("net.minecraft.client.Options$FieldAccess"));
        process.setAccessible(true);
        Field valuesField = OptionInstance.class.getDeclaredField("values");
        valuesField.setAccessible(true);
        Gson gson = new Gson();
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0]), StandardCharsets.UTF_8))) {
            Class<?> fa = Class.forName("net.minecraft.client.Options$FieldAccess");
            Object access = Proxy.newProxyInstance(fa.getClassLoader(), new Class<?>[]{fa}, (proxy, method, a) -> {
                String name = (String) a[0];
                Class<?> rt = method.getReturnType();
                if (a.length == 2 && a[1] instanceof OptionInstance<?> oi) {
                    Object vs = valuesField.get(oi);
                    var encoded = ((OptionInstance<Object>) oi).codec().encodeStart(JsonOps.INSTANCE, oi.get()).result().orElse(null);
                    if (vs instanceof OptionInstance.Enum<?> en) {
                        StringBuilder sb = new StringBuilder();
                        for (Object v : en.values()) {
                            sb.append(gson.toJson(((com.mojang.serialization.Codec<Object>) en.codec()).encodeStart(JsonOps.INSTANCE, v).result().orElse(null))).append(',');
                        }
                        out.println("enum " + name + " " + sb);
                    }
                    if (vs instanceof OptionInstance.SliderableEnum<?> en) {
                        StringBuilder sb = new StringBuilder();
                        for (Object v : en.values()) {
                            sb.append(gson.toJson(((com.mojang.serialization.Codec<Object>) en.codec()).encodeStart(JsonOps.INSTANCE, v).result().orElse(null))).append(',');
                        }
                        out.println("enum " + name + " " + sb);
                    }
                    out.println("option " + name + " " + vs.getClass().getSimpleName() + " " + describe(vs) + " => " + gson.toJson(encoded));
                    return null;
                }
                if (a.length == 2) {
                    out.println("plain " + name + " " + rt.getSimpleName() + " => " + a[1]);
                    return a[1];
                }
                if (a.length == 4) {
                    Object conv = ((Function<Object, String>) a[3]).apply(a[1]);
                    out.println("custom " + name + " => " + conv);
                    return a[1];
                }
                return null;
            });
            process.invoke(options, access);
        }
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            Field map = com.mojang.blaze3d.platform.InputConstants.Type.class.getDeclaredField("map");
            map.setAccessible(true);
            for (var type : com.mojang.blaze3d.platform.InputConstants.Type.values()) {
                var m = (it.unimi.dsi.fastutil.ints.Int2ObjectMap<com.mojang.blaze3d.platform.InputConstants.Key>) map.get(type);
                var entries = new TreeMap<Integer, String>();
                for (var e : m.int2ObjectEntrySet()) {
                    entries.put(e.getIntKey(), e.getValue().getName());
                }
                for (var e : entries.entrySet()) {
                    out.println(type.name().toLowerCase() + " " + e.getKey() + " " + e.getValue());
                }
            }
        }
        System.exit(0);
    }

    static String describe(Object vs) throws Exception {
        if (vs instanceof Record r) {
            StringBuilder sb = new StringBuilder("{");
            for (var c : r.getClass().getRecordComponents()) {
                Object v = c.getAccessor().invoke(r);
                String s = String.valueOf(v);
                if (v instanceof List<?> l) {
                    s = l.toString();
                } else if (v != null && s.contains("$$Lambda")) {
                    s = "<fn>";
                }
                if (c.getName().equals("codec") || c.getName().equals("valueSetter") || c.getName().equals("altCondition")
                    || c.getName().equals("validateValue") || c.getName().equals("maxSupplier") || c.getName().equals("values") && !(v instanceof List)) {
                    if (c.getName().equals("maxSupplier")) {
                        s = "<supplier " + ((java.util.function.IntSupplier) v).getAsInt() + ">";
                    } else if (!c.getName().equals("values")) {
                        s = "<fn>";
                    }
                }
                sb.append(c.getName()).append('=').append(s).append(' ');
            }
            return sb.append('}').toString();
        }
        return vs.toString();
    }
}
