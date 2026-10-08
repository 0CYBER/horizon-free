package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import it.unimi.dsi.fastutil.objects.Object2ObjectOpenHashMap;

/** Executes the actual pinned fastutil map, including live MapEntry handles. */
public final class VanillaObjectOpenMapOracle {
    record Key(int id, int hash, int object) {
        @Override public int hashCode() { return hash; }
        @Override public boolean equals(Object o) { return o instanceof Key k && id == k.id; }
    }
    static final class CallbackFailure extends RuntimeException {}
    static String desc(Key key) { return key == null ? "null" : key.id() + ":" + key.object(); }
    static Integer value(JsonElement v) { return v.isJsonNull() ? null : v.getAsInt(); }
    static String snapshot(Object2ObjectOpenHashMap<Key, Integer> map) {
        var entries = new StringJoiner(",");
        map.forEach((k, v) -> entries.add(desc(k) + "=" + v));
        return "map " + map.size() + " " + map.isEmpty() + " | " + entries;
    }
    static String mutation(Object2ObjectOpenHashMap<Key, Integer> map, List<Key> keys, JsonArray op) {
        int index = op.get(1).getAsInt();
        Key key = index < 0 ? null : keys.get(index);
        return switch (op.get(0).getAsString()) {
            case "put" -> "put " + map.put(key, value(op.get(2)));
            case "remove" -> "remove " + map.remove(key);
            case "contains" -> "contains " + map.containsKey(key);
            case "get" -> "get " + map.get(key);
            case "clear" -> { map.clear(); yield "clear"; }
            default -> throw new AssertionError();
        };
    }
    public static void main(String[] args) throws Exception {
        Path dir = Path.of(args[1]);
        try (var out = new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0] + ".txt")));
             var lines = Files.lines(dir.resolve("inputs.txt"))) {
            for (var line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                var keys = new ArrayList<Key>();
                for (var h : f.getAsJsonArray("hashes")) {
                    int i = keys.size();
                    keys.add(new Key(f.getAsJsonArray("ids").get(i).getAsInt(), h.getAsInt(), i));
                }
                var map = new Object2ObjectOpenHashMap<Key, Integer>();
                Iterator<Map.Entry<Key, Integer>> cursor = map.entrySet().iterator();
                var entries = new ArrayList<Map.Entry<Key, Integer>>();
                for (var ov : f.getAsJsonArray("ops")) {
                    var op = ov.getAsJsonArray();
                    try {
                        switch (op.get(0).getAsString()) {
                            case "put", "remove", "contains", "get", "clear" -> out.println(mutation(map, keys, op));
                            case "cursor" -> { cursor = map.entrySet().iterator(); out.println("cursor"); }
                            case "next", "force-next" -> {
                                if (cursor.hasNext() || op.get(0).getAsString().equals("force-next")) {
                                    var entry = cursor.next();
                                    entries.add(entry);
                                    out.println("next " + (entries.size() - 1) + " " + desc(entry.getKey()) + "=" + entry.getValue());
                                } else out.println("next none");
                            }
                            case "delete" -> { cursor.remove(); out.println("delete"); }
                            case "entry-key" -> out.println("entry-key " + desc(entries.get(op.get(1).getAsInt()).getKey()));
                            case "entry-value" -> out.println("entry-value " + entries.get(op.get(1).getAsInt()).getValue());
                            case "entry-set" -> out.println("entry-set " + entries.get(op.get(1).getAsInt()).setValue(value(op.get(2))));
                            case "each" -> {
                                int[] calls = {0};
                                map.forEach((k, v) -> {
                                    int call = calls[0];
                                    out.println("visit " + call + " " + desc(k) + "=" + v);
                                    var actions = op.get(1).getAsJsonArray();
                                    if (call < actions.size()) {
                                        for (var av : actions.get(call).getAsJsonArray()) {
                                            var action = av.getAsJsonArray();
                                            if (action.get(0).getAsString().equals("fail")) throw new CallbackFailure();
                                            out.println("callback " + mutation(map, keys, action));
                                            out.println(snapshot(map));
                                        }
                                    }
                                    calls[0]++;
                                });
                                out.println("each");
                            }
                            default -> throw new AssertionError();
                        }
                    } catch (CallbackFailure e) { out.println("error callback");
                    } catch (IllegalStateException e) { out.println("error current");
                    } catch (NoSuchElementException e) { out.println("next none");
                    } catch (NullPointerException e) { out.println("error null");
                    } catch (IndexOutOfBoundsException e) { out.println("error bounds"); }
                    out.println(snapshot(map) + " | cursor " + cursor.hasNext());
                }
            }
        }
    }
}
