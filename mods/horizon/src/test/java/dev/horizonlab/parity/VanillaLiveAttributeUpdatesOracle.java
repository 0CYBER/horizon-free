package dev.horizonlab.parity;

import static dev.horizonlab.parity.VanillaEntityAttributeOracle.*;
import com.google.gson.*;
import com.google.common.collect.ImmutableList;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.effect.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.attributes.*;
import net.minecraft.world.level.Level;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.waypoints.ServerWaypointManager;
import net.minecraft.world.waypoints.WaypointTransmitter;
import net.minecraft.network.syncher.*;

/** Native entity updates and notifications with actual registered attribute maps. */
public final class VanillaLiveAttributeUpdatesOracle {
    static Object unsafe;
    static Method allocate, refresh;
    static Field healthData, absorption, attributesField, effectDirty;
    static EntityDataAccessor<Float> healthKey;
    static Constructor<SynchedEntityData> dataCtor;
    static <T> T make(Class<T> type) throws Exception { return type.cast(allocate.invoke(unsafe, type)); }
    static float single(JsonElement v) { return Float.intBitsToFloat((int)v.getAsLong()); }
    static long floatBits(float value) { return Integer.toUnsignedLong(Float.floatToIntBits(value)); }
    static final class Stop extends RuntimeException {}
    static final class Data {
        VanillaLiveAttributesOracle.Data[] maps = new VanillaLiveAttributesOracle.Data[2];
        VanillaLiveAttributesOracle.Effect effect = new VanillaLiveAttributesOracle.Effect();
        SynchedEntityData[] stores = new SynchedEntityData[2];
        List<SynchedEntityData.DataItem<Float>> items = new ArrayList<>();
        Manager[] managers = new Manager[2];
        Actor actor; EffectLevel level; PrintWriter out; JsonObject op;
        int selection, direct, data, manager, calls;
        boolean server = true, client;
        Holder<Attribute> key(JsonObject op) {
            int index = op.get("attribute").getAsInt();
            return index < 0 ? null : attributes.get(index);
        }
        void controls(JsonObject c) throws Exception {
            if (c.has("selection")) selection = c.get("selection").getAsInt();
            if (c.has("direct")) direct = c.get("direct").getAsInt();
            if (c.has("data")) data = c.get("data").getAsInt();
            if (c.has("manager")) manager = c.get("manager").getAsInt();
            if (c.has("server")) server = c.get("server").getAsBoolean();
            if (c.has("client")) client = c.get("client").getAsBoolean();
            if (c.has("absorption")) absorption.setFloat(actor, single(c.get("absorption")));
            healthData.set(actor, stores[data]);
            attributesField.set(actor, maps[direct].attributes);
        }
        void event(String name) {
            int call = calls++;
            out.println("event " + call + " " + name);
            if (call == op.get("fail").getAsInt()) throw new Stop();
            if (call == op.get("mutate").getAsInt()) {
                try {
                    controls(op.getAsJsonObject("changes"));
                    for (var action : op.getAsJsonArray("actions")) raw(action.getAsJsonObject());
                } catch (RuntimeException e) { throw e; }
                catch (Exception e) { throw new AssertionError(e); }
            }
        }
        String raw(JsonObject op) throws Exception {
            var map = maps[op.get("target").getAsInt()];
            var key = key(op);
            String mode = op.get("mode").getAsString();
            var modifier = new AttributeModifier(Identifier.parse(op.get("id").getAsString()), number(op.get("amount")), AttributeModifier.Operation.values()[op.get("operation").getAsInt()]);
            switch (mode) {
                case "controls": controls(op.getAsJsonObject("changes")); return "ok";
                case "ensure": return map.reference(map.attributes.getInstance(key));
                case "reset": return Boolean.toString(map.attributes.resetBaseValue(key));
                case "define": effect.addAttributeModifier(key, modifier.id(), modifier.amount(), modifier.operation()); return "ok";
                case "effect-add": effect.addAttributeModifiers(map.attributes, op.get("amplifier").getAsInt()); return "ok";
                case "effect-remove": effect.removeAttributeModifiers(map.attributes); return "ok";
                case "clear-updates": map.attributes.getAttributesToUpdate().clear(); return "ok";
                case "clear-sync": map.attributes.getAttributesToSync().clear(); return "ok";
                case "null-update": map.attributes.getAttributesToUpdate().add(null); return "ok";
                case "dirty": effectDirty.setBoolean(actor, op.get("flag").getAsBoolean()); return "ok";
                default:
                    var instance = map.attributes.getInstance(key);
                    if (instance == null) return "absent";
                    switch (mode) {
                        case "set": instance.setBaseValue(number(op.get("base"))); break;
                        case "update": instance.addOrUpdateTransientModifier(modifier); break;
                        case "permanent": instance.addPermanentModifier(modifier); break;
                        case "remove": return Boolean.toString(instance.removeModifier(modifier.id()));
                        case "clear": instance.removeModifiers(); break;
                        default: throw new AssertionError(mode);
                    }
                    return "ok";
            }
        }
        void state() throws Exception {
            out.println("controls " + selection + " " + direct + " " + data + " " + manager + " " + server + " " + client + " " + calls + " " + effectDirty.getBoolean(actor));
            for (int i = 0; i < 2; i++) out.println("health " + i + " " + floatBits(items.get(i).getValue()) + " " + stores[i].isDirty() + " " + items.get(i).isDirty() + " " + items.get(i).isSetToDefault());
            out.println("absorption " + floatBits(absorption.getFloat(actor)));
            for (int i = 0; i < 2; i++) { out.println("attributes " + i); maps[i].state(out); }
        }
    }
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {
        Data d; Actor() { super(null, null); }
        @Override public AttributeMap getAttributes() { d.event("attributes"); return d.maps[d.selection].attributes; }
        @Override public double getAttributeValue(Holder<Attribute> h) { d.event("value " + (h == null ? "null" : h.getRegisteredName())); return super.getAttributeValue(h); }
        @Override public float getHealth() { d.event("health"); return super.getHealth(); }
        @Override public void setHealth(float v) { d.event("set-health " + floatBits(v)); super.setHealth(v); }
        @Override public float getAbsorptionAmount() { d.event("absorption"); return super.getAbsorptionAmount(); }
        @Override protected void internalSetAbsorptionAmount(float v) { d.event("store-absorption " + floatBits(v)); super.internalSetAbsorptionAmount(v); }
        @Override public void onSyncedDataUpdated(EntityDataAccessor<?> key) { if (key != healthKey) throw new AssertionError(); d.event("health-updated"); }
        @Override public void refreshDimensions() { d.event("dimensions"); }
        @Override public Level level() { d.event("level"); return d.server ? d.level : null; }
        @Override protected void onAttributeUpdated(Holder<Attribute> h) { d.event("updated " + (h == null ? "null" : h.getRegisteredName())); super.onAttributeUpdated(h); }
        void added(MobEffectInstance e) { super.onEffectAdded(e, null); }
        void updated(MobEffectInstance e, boolean reapply) { super.onEffectUpdated(e, reapply, null); }
        void removed(List<MobEffectInstance> es) { super.onEffectsRemoved(es); }
    }
    static final class EffectLevel extends ServerLevel {
        Data d; EffectLevel() { super(null,null,null,null,null,null,false,0L,List.of(),false); }
        @Override public boolean isClientSide() { d.event("client"); return d.client; }
        @Override public ServerWaypointManager getWaypointManager() { d.event("manager"); return d.managers[d.manager]; }
    }
    static final class Manager extends ServerWaypointManager {
        Data d; int id;
        @Override public void trackWaypoint(WaypointTransmitter actor) { if (actor != d.actor) throw new AssertionError(); d.event("waypoint " + id + " true"); }
        @Override public void untrackWaypoint(WaypointTransmitter actor) { if (actor != d.actor) throw new AssertionError(); d.event("waypoint " + id + " false"); }
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion(); net.minecraft.server.Bootstrap.bootStrap();
        for (var a : BuiltInRegistries.ATTRIBUTE) attributes.add(BuiltInRegistries.ATTRIBUTE.wrapAsHolder(a));
        Path dir = Path.of(args[1]);
        if (args[0].startsWith("export-")) { Files.writeString(dir.resolve("catalog-" + args[0].substring(7) + ".json"), export().toString()); return; }
        var u = Class.forName("sun.misc.Unsafe"); unsafe = field(u, "theUnsafe").get(null); allocate = u.getMethod("allocateInstance", Class.class);
        healthData = field(Entity.class, "entityData"); absorption = field(LivingEntity.class, "absorptionAmount"); attributesField = field(LivingEntity.class, "attributes"); effectDirty = field(LivingEntity.class, "effectsDirty");
        var passengers = field(Entity.class, "passengers");
        refresh = LivingEntity.class.getDeclaredMethod("refreshDirtyAttributes"); refresh.setAccessible(true);
        healthKey = (EntityDataAccessor<Float>)field(LivingEntity.class, "DATA_HEALTH_ID").get(null);
        dataCtor = SynchedEntityData.class.getDeclaredConstructor(SyncedDataHolder.class, SynchedEntityData.DataItem[].class); dataCtor.setAccessible(true);
        VanillaLiveAttributesOracle.storage = field(AttributeMap.class, "attributes"); VanillaLiveAttributesOracle.dirty = field(AttributeInstance.class, "dirty"); VanillaLiveAttributesOracle.cached = field(AttributeInstance.class, "cachedValue");
        try (var out = new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0] + ".txt")));
             var enriched = new PrintWriter(Files.newBufferedWriter(dir.resolve("inputs-" + args[0] + ".txt")));
             var lines = Files.lines(dir.resolve("inputs.txt"))) {
            for (var line : (Iterable<String>)lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject(); var d = new Data(); d.out = out;
                d.actor = make(Actor.class); d.actor.d = d; d.level = make(EffectLevel.class); d.level.d = d;
                var entity = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(f.get("entity").getAsString()));
                for (int i = 0; i < 2; i++) {
                    var map = new VanillaLiveAttributesOracle.Data(); d.maps[i] = map;
                    map.attributes = new VanillaLiveAttributesOracle.Attrs(map, supplier(entity));
                    map.map = (Map<Holder<Attribute>,AttributeInstance>)VanillaLiveAttributesOracle.storage.get(map.attributes);
                    d.managers[i] = make(Manager.class); d.managers[i].d = d; d.managers[i].id = i;
                    var items = new SynchedEntityData.DataItem<?>[healthKey.id() + 1];
                    for (int j = 0; j < items.length; j++) items[j] = new SynchedEntityData.DataItem<Float>(new EntityDataAccessor<Float>(j, EntityDataSerializers.FLOAT), 0f);
                    var health = new SynchedEntityData.DataItem<Float>(healthKey, single(f.getAsJsonArray("health").get(i)));
                    d.items.add(health); items[healthKey.id()] = health; d.stores[i] = dataCtor.newInstance(d.actor, items);
                }
                effectDirty.setBoolean(d.actor, true); passengers.set(d.actor, ImmutableList.of()); absorption.setFloat(d.actor, single(f.get("absorption"))); d.controls(new JsonObject());
                for (var op : f.getAsJsonArray("setup")) d.raw(op.getAsJsonObject());
                var effect = Holder.<MobEffect>direct(d.effect);
                for (var ov : f.getAsJsonArray("ops")) {
                    d.op = ov.getAsJsonObject(); d.calls = 0; String result = "ok";
                    var op = d.op; String mode = op.get("mode").getAsString();
                    try {
                        switch (mode) {
                            case "value": result = bits(d.actor.getAttributeValue(d.key(op))); break;
                            case "base": result = bits(d.actor.getAttributeBaseValue(d.key(op))); break;
                            case "set-health": d.actor.setHealth(single(op.get("value"))); break;
                            case "set-absorption": d.actor.setAbsorptionAmount(single(op.get("value"))); break;
                            case "heal": d.actor.heal(single(op.get("value"))); break;
                            case "attribute-update": d.actor.onAttributeUpdated(d.key(op)); break;
                            case "refresh": try { refresh.invoke(d.actor); } catch (InvocationTargetException e) { if (e.getCause() instanceof RuntimeException x) throw x; throw new AssertionError(e.getCause()); } break;
                            case "notify-add": d.actor.added(new MobEffectInstance(effect,200,op.get("amplifier").getAsInt())); break;
                            case "notify-update": d.actor.updated(new MobEffectInstance(effect,200,op.get("amplifier").getAsInt()),op.get("reapply").getAsBoolean()); break;
                            case "notify-remove": d.actor.removed(Collections.nCopies(op.get("count").getAsInt(),new MobEffectInstance(effect,200,op.get("amplifier").getAsInt()))); break;
                            case "pack": var packed = d.stores[op.get("target").getAsInt()].packDirty(); result = packed == null ? "none" : Long.toString(floatBits((Float)packed.getFirst().value())); break;
                            default: result = d.raw(op);
                        }
                    } catch (Stop e) { result = "error callback"; }
                    catch (NullPointerException e) { result = "error null"; }
                    catch (IndexOutOfBoundsException e) { result = "error bounds"; }
                    catch (NoSuchElementException e) { result = "error empty"; }
                    catch (IllegalArgumentException e) { result = "error argument"; }
                    out.println("result " + result); d.state();
                }
                var hashes = new JsonArray(); for (var key : attributes) hashes.add(key.hashCode()); f.add("holder_hashes", hashes);
                var instances = new JsonArray(); for (var map : d.maps) { hashes = new JsonArray(); for (var a : map.instances) hashes.add(System.identityHashCode(a)); instances.add(hashes); } f.add("instance_hashes", instances);
                enriched.println(f);
            }
            if (out.checkError() || enriched.checkError()) throw new IOException("cannot write native trace");
        }
    }
}
