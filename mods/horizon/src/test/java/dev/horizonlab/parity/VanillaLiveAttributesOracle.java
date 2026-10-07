package dev.horizonlab.parity;

import static dev.horizonlab.parity.VanillaEntityAttributeOracle.*;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.Field;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.effect.*;
import net.minecraft.world.entity.ai.attributes.*;

/** Real registered suppliers, maps, instance caches, modifiers and pending sets. */
public final class VanillaLiveAttributesOracle {
    static Field storage, dirty, cached;
    static final class Effect extends MobEffect {
        Effect() { super(MobEffectCategory.BENEFICIAL, 0); }
    }
    static final class Attrs extends AttributeMap {
        final Data data;
        Attrs(Data data, AttributeSupplier supplier) { super(supplier); this.data = data; }
        @Override public AttributeInstance getInstance(Holder<Attribute> key) {
            var instance = super.getInstance(key);
            data.track(instance);
            return instance;
        }
    }
    static final class Data {
        final List<AttributeInstance> instances = new ArrayList<>();
        final List<AttributeModifier> modifiers = new ArrayList<>();
        Attrs attributes;
        Map<Holder<Attribute>, AttributeInstance> map;
        void track(AttributeInstance instance) {
            if (instance != null && !instances.contains(instance)) instances.add(instance);
        }
        String reference(AttributeInstance instance) {
            if (instance == null) return "null";
            int id = instances.indexOf(instance);
            if (id < 0) throw new AssertionError("untracked instance");
            return Integer.toString(id);
        }
        String references(Collection<AttributeInstance> values) {
            var result = new StringJoiner(",");
            for (var instance : values) result.add(reference(instance));
            return result.toString();
        }
        String modifierReferences(Collection<AttributeModifier> values) {
            var result = new StringJoiner(",");
            for (var modifier : values) {
                int id = -1;
                for (int i = 0; i < modifiers.size(); i++) if (modifiers.get(i) == modifier) { id = i; break; }
                if (id < 0) { id = modifiers.size(); modifiers.add(modifier); }
                result.add(id + ":" + modifier.id() + ":" + bits(modifier.amount()) + ":" + modifier.operation().ordinal());
            }
            return result.toString();
        }
        void state(PrintWriter out) throws Exception {
            var entries = new StringJoiner(",");
            for (var entry : map.entrySet()) entries.add((entry.getKey() == null ? "null" : entry.getKey().getRegisteredName()) + "=" + reference(entry.getValue()));
            out.println("map " + entries);
            out.println("updates " + references(attributes.getAttributesToUpdate()));
            out.println("sync " + references(attributes.getAttributesToSync()));
            for (int i = 0; i < instances.size(); i++) {
                var instance = instances.get(i);
                out.println("instance " + i + " " + instance.getAttribute().getRegisteredName() + " " + bits(instance.getBaseValue()) + " " + dirty.getBoolean(instance) + " " + bits(cached.getDouble(instance)) + " | " + modifierReferences(instance.getModifiers()) + " | " + modifierReferences(instance.getPermanentModifiers()));
            }
        }
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        for (var a : BuiltInRegistries.ATTRIBUTE) attributes.add(BuiltInRegistries.ATTRIBUTE.wrapAsHolder(a));
        Path dir = Path.of(args[1]);
        if (args[0].startsWith("export-")) {
            Files.writeString(dir.resolve("catalog-" + args[0].substring(7) + ".json"), export().toString());
            return;
        }
        storage = field(AttributeMap.class, "attributes");
        dirty = field(AttributeInstance.class, "dirty");
        cached = field(AttributeInstance.class, "cachedValue");
        try (var out = new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0] + ".txt")));
             var enriched = new PrintWriter(Files.newBufferedWriter(dir.resolve("inputs-" + args[0] + ".txt")));
             var lines = Files.lines(dir.resolve("inputs.txt"))) {
            for (var line : (Iterable<String>)lines::iterator) {
                var fixture = JsonParser.parseString(line).getAsJsonObject();
                var entity = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(fixture.get("entity").getAsString()));
                var data = new Data();
                data.attributes = new Attrs(data, supplier(entity));
                data.map = (Map<Holder<Attribute>, AttributeInstance>)storage.get(data.attributes);
                var effect = new Effect();
                for (var value : fixture.getAsJsonArray("ops")) {
                    var op = value.getAsJsonObject();
                    int index = op.get("attribute").getAsInt();
                    var key = index < 0 ? null : attributes.get(index);
                    String mode = op.get("mode").getAsString();
                    var modifier = new AttributeModifier(Identifier.parse(op.get("id").getAsString()), number(op.get("amount")), AttributeModifier.Operation.values()[op.get("operation").getAsInt()]);
                    String result = "ok";
                    try {
                        switch (mode) {
                            case "value" -> result = bits(data.attributes.getValue(key));
                            case "base" -> result = bits(data.attributes.getBaseValue(key));
                            case "has" -> result = Boolean.toString(data.attributes.hasAttribute(key));
                            case "has_modifier" -> result = Boolean.toString(data.attributes.hasModifier(key, modifier.id()));
                            case "modifier" -> result = bits(data.attributes.getModifierValue(key, modifier.id()));
                            case "ensure" -> result = data.reference(data.attributes.getInstance(key));
                            case "reset" -> result = Boolean.toString(data.attributes.resetBaseValue(key));
                            case "ack_update" -> data.attributes.getAttributesToUpdate().remove(data.map.get(key));
                            case "ack_sync" -> data.attributes.getAttributesToSync().remove(data.map.get(key));
                            case "clear_updates" -> data.attributes.getAttributesToUpdate().clear();
                            case "clear_sync" -> data.attributes.getAttributesToSync().clear();
                            case "define" -> effect.addAttributeModifier(key, modifier.id(), modifier.amount(), modifier.operation());
                            case "effect-add" -> effect.addAttributeModifiers(data.attributes, op.get("amplifier").getAsInt());
                            case "effect-remove" -> effect.removeAttributeModifiers(data.attributes);
                            default -> {
                                var instance = data.attributes.getInstance(key);
                                if (instance == null) result = "absent";
                                else switch (mode) {
                                    case "set" -> instance.setBaseValue(number(op.get("base")));
                                    case "add" -> instance.addTransientModifier(modifier);
                                    case "update" -> instance.addOrUpdateTransientModifier(modifier);
                                    case "permanent" -> instance.addPermanentModifier(modifier);
                                    case "remove" -> result = Boolean.toString(instance.removeModifier(modifier.id()));
                                    case "clear" -> instance.removeModifiers();
                                    default -> throw new AssertionError(mode);
                                }
                            }
                        }
                    } catch (NullPointerException e) {
                        result = "error null";
                    } catch (IllegalArgumentException e) {
                        result = "error " + (mode.equals("modifier") ? (data.attributes.hasAttribute(key) ? "modifier" : "attribute") : (mode.equals("value") || mode.equals("base") ? "attribute" : "duplicate"));
                    }
                    out.println("result " + result);
                    data.state(out);
                }
                var hashes = new JsonArray();
                for (var key : attributes) hashes.add(key.hashCode());
                fixture.add("holder_hashes", hashes);
                hashes = new JsonArray();
                for (var instance : data.instances) hashes.add(System.identityHashCode(instance));
                fixture.add("instance_hashes", hashes);
                enriched.println(fixture);
            }
            if (out.checkError() || enriched.checkError()) throw new IOException("cannot write native trace");
        }
    }
}
