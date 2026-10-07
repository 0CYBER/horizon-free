package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.Holder;
import net.minecraft.resources.Identifier;
import net.minecraft.world.effect.*;
import net.minecraft.world.entity.ai.attributes.*;

/** Native MobEffect modifier methods and AttributeInstance storage/callbacks. */
public final class VanillaEffectAttributeModifiersOracle {
    static Field definitions, dirty, cached, templateId, templateAmount, templateOperation;
    static Field field(Class<?> c, String name) throws Exception {
        var f = c.getDeclaredField(name); f.setAccessible(true); return f;
    }
    static double number(JsonElement v) { return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(), 16)); }
    static String bits(double v) { return String.format("%016x", Double.doubleToLongBits(v)); }
    static String desc(AttributeModifier m) { return m.id() + ":" + bits(m.amount()) + ":" + m.operation().ordinal(); }
    static final class Stop extends RuntimeException {}
    static final class Basic extends Attribute {
        int hash;
        Basic(double value, int hash) { super("horizon.fixture", value); this.hash = hash; }
        @Override public int hashCode() { return hash; }
    }
    static final class Effect extends MobEffect { Effect() { super(MobEffectCategory.BENEFICIAL, 0); } }
    record Template(Identifier id, double amount, AttributeModifier.Operation operation) {
        AttributeModifier create(int amplifier) { return new AttributeModifier(id, amount * (double)(amplifier + 1), operation); }
    }
    static final class Data {
        List<Holder<Attribute>> keys = new ArrayList<>();
        List<Template> templates = new ArrayList<>();
        List<AttributeModifier> pool = new ArrayList<>();
        int[] routes; Attr[] instances; Attrs attributes; Effect effect = new Effect();
        Map<Holder<Attribute>, Object> defs; JsonObject op; int calls; boolean changed; PrintWriter out;
        Holder<Attribute> key(JsonObject op) { int k = op.get("key").getAsInt(); return k < 0 ? null : keys.get(k); }
        String keyDesc(Holder<Attribute> key) { return key == null ? "null" : Integer.toString(keys.indexOf(key)); }
        int modifier(AttributeModifier m) { pool.add(m); return pool.size() - 1; }
        String refs(Collection<AttributeModifier> mods) {
            var out = new StringJoiner(",");
            for (var m : mods) {
                int id = -1; for (int i = 0; i < pool.size(); i++) if (pool.get(i) == m) { id = i; break; }
                if (id < 0) throw new AssertionError("unknown modifier");
                out.add(Integer.toString(id));
            }
            return out.toString();
        }
        void definitions() {
            var result = new StringJoiner(",");
            defs.forEach((key, t) -> {
                try { result.add(keyDesc(key) + "=" + (t == null ? "null" : templateId.get(t) + ":" + bits(templateAmount.getDouble(t)) + ":" + ((AttributeModifier.Operation)templateOperation.get(t)).ordinal())); }
                catch (IllegalAccessException e) { throw new AssertionError(e); }
            });
            out.println("definitions " + defs.size() + " | " + result);
        }
        void state(int id, AttributeInstance a) {
            try { out.println("instance " + id + " " + bits(a.getBaseValue()) + " " + dirty.getBoolean(a) + " " + bits(cached.getDouble(a)) + " | " + refs(a.getModifiers()) + " | " + refs(a.getPermanentModifiers())); }
            catch (IllegalAccessException e) { throw new AssertionError(e); }
        }
        void raw(JsonObject op) {
            switch (op.get("op").getAsString()) {
                case "define" -> {
                    var t = templates.get(op.get("template").getAsInt());
                    if (effect.addAttributeModifier(key(op), t.id(), t.amount(), t.operation()) != effect) throw new AssertionError();
                }
                case "null" -> defs.put(key(op), null);
                case "drop" -> defs.remove(key(op));
                case "clear" -> defs.clear();
                case "route" -> { var k = key(op); routes[k == null ? keys.size() : keys.indexOf(k)] = op.get("target").getAsInt(); }
                default -> throw new AssertionError();
            }
        }
        void event(String name) {
            int call = calls++;
            out.println("event " + call + " " + name); definitions();
            if (call == op.get("fail").getAsInt()) throw new Stop();
            if (call == op.get("mutate").getAsInt()) {
                changed = true;
                for (var a : op.getAsJsonArray("mutations")) raw(a.getAsJsonObject());
            }
        }
        void changed(AttributeInstance a) {
            int id = ((Attr)a).id; state(id, a); event("dirty " + id);
            if (op.get("read").getAsBoolean()) out.println("observed " + bits(a.getValue()));
        }
    }
    static final class Attrs extends AttributeMap {
        Data d;
        Attrs(Data d) { super(null); this.d = d; }
        @Override public AttributeInstance getInstance(Holder<Attribute> h) {
            d.event("get " + d.keyDesc(h));
            int route = d.routes[h == null ? d.keys.size() : d.keys.indexOf(h)];
            return route < 0 ? null : d.instances[route];
        }
    }
    static final class Attr extends AttributeInstance {
        Data d; int id;
        Attr(Data d, int id, double value) { super(Holder.direct(new Basic(value, id)), d::changed); this.d = d; this.id = id; }
        @Override public boolean removeModifier(Identifier id) { d.event("remove " + this.id + " " + id); return super.removeModifier(id); }
        @Override public void addPermanentModifier(AttributeModifier modifier) {
            int m = d.modifier(modifier); d.event("permanent " + id + " " + m + " " + desc(modifier));
            super.addPermanentModifier(modifier);
        }
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion(); net.minecraft.server.Bootstrap.bootStrap();
        definitions = field(MobEffect.class, "attributeModifiers");
        dirty = field(AttributeInstance.class, "dirty"); cached = field(AttributeInstance.class, "cachedValue");
        var template = Class.forName("net.minecraft.world.effect.MobEffect$AttributeTemplate");
        templateId = field(template, "id"); templateAmount = field(template, "amount"); templateOperation = field(template, "operation");
        Path dir = Path.of(args[1]);
        try (var out = new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0] + ".txt")));
             var lines = Files.lines(dir.resolve("inputs.txt"))) {
            for (var line : (Iterable<String>)lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject(); var d = new Data(); d.out = out;
                d.defs = (Map<Holder<Attribute>, Object>)definitions.get(d.effect);
                // Direct is a two-component record (value, components). Solve
                // its record hash for the requested holder hash so collision
                // fixtures target the actual keys, not only their values.
                int componentsHash = Holder.direct(new Basic(0, 0)).hashCode();
                for (var h : f.getAsJsonArray("hashes")) {
                    int requested = h.getAsInt();
                    var key = Holder.<Attribute>direct(new Basic(0, (requested - componentsHash) * 0xbdef7bdf));
                    if (key.hashCode() != requested) throw new AssertionError("holder hash fixture");
                    d.keys.add(key);
                }
                for (var v : f.getAsJsonArray("templates")) {
                    var t = v.getAsJsonObject();
                    d.templates.add(new Template(Identifier.parse(t.get("id").getAsString()), number(t.get("amount")), AttributeModifier.Operation.values()[t.get("operation").getAsInt()]));
                }
                d.routes = new int[d.keys.size() + 1];
                for (int i = 0; i < d.routes.length; i++) d.routes[i] = f.getAsJsonArray("routes").get(i).getAsInt();
                d.instances = new Attr[f.getAsJsonArray("defaults").size()];
                for (int i = 0; i < d.instances.length; i++) d.instances[i] = new Attr(d, i, number(f.getAsJsonArray("defaults").get(i)));
                d.attributes = new Attrs(d);
                for (var v : f.getAsJsonArray("setup")) d.raw(v.getAsJsonObject());
                for (var v : f.getAsJsonArray("ops")) {
                    d.op = v.getAsJsonObject(); d.calls = 0; d.changed = false; String result = "ok";
                    int amp = d.op.get("amplifier").getAsInt();
                    try {
                        switch (d.op.get("op").getAsString()) {
                            case "add" -> d.effect.addAttributeModifiers(d.attributes, amp);
                            case "remove" -> d.effect.removeAttributeModifiers(d.attributes);
                            case "create" -> d.effect.createModifiers(amp, (key, modifier) -> {
                                int id = d.modifier(modifier); d.event("consume " + d.keyDesc(key) + " " + id + " " + desc(modifier));
                            });
                            case "seed" -> d.instances[d.op.get("target").getAsInt()].addPermanentModifier(d.templates.get(d.op.get("template").getAsInt()).create(amp));
                            case "value" -> { int id = d.op.get("target").getAsInt(); out.println("value " + id + " " + bits(d.instances[id].getValue())); }
                            default -> d.raw(d.op);
                        }
                    } catch (Stop e) { result = "callback";
                    } catch (NullPointerException e) { result = "null";
                    } catch (IndexOutOfBoundsException e) { result = "bounds";
                    } catch (NoSuchElementException e) { result = "empty";
                    } catch (IllegalArgumentException e) { result = "duplicate"; }
                    out.println("result " + result); out.println("calls " + d.calls + " " + d.changed); d.definitions();
                    for (int i = 0; i < d.instances.length; i++) d.state(i, d.instances[i]);
                }
            }
        }
    }
}
