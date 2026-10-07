package dev.horizonlab.parity;

import static dev.horizonlab.parity.VanillaEffectParticlesOracle.*;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.world.effect.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.Level;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.network.syncher.*;

/** Inherited native effect synchronization and real heterogeneous entity data. */
public final class VanillaEffectSyncOracle {
    static Object unsafe; static Method allocate, dirtyUpdate, particlesUpdate, glowingUpdate;
    static Field dataField, dirtyField, tagField, effectsField, visibleField, ambientField;
    static EntityDataAccessor<Byte> flagsKey;
    static EntityDataAccessor<Boolean> ambientKey;
    static EntityDataAccessor<List<ParticleOptions>> particlesKey;
    static Constructor<SynchedEntityData> dataCtor;
    static List<Holder<MobEffect>> keys = new ArrayList<>();
    static <T> T make(Class<T> c) throws Exception { return c.cast(allocate.invoke(unsafe, c)); }
    static Method method(String name) throws Exception { var m = LivingEntity.class.getDeclaredMethod(name); m.setAccessible(true); return m; }
    static void invoke(Method m, Actor actor) throws Exception { try { m.invoke(actor); } catch (InvocationTargetException e) { if (e.getCause() instanceof RuntimeException x) throw x; throw new AssertionError(e.getCause()); } }
    static final class Stop extends RuntimeException {}
    static final class Data {
        Actor actor; EffectLevel level; PrintWriter out; JsonObject op;
        SynchedEntityData[] tables = new SynchedEntityData[2];
        List<SynchedEntityData.DataItem<?>>[] items;
        Map<Holder<MobEffect>, MobEffectInstance> effects = new HashMap<>();
        List<MobEffectInstance> pool = new ArrayList<>();
        List<ParticleOptions> particles = new ArrayList<>();
        int data, calls; boolean client;
        String particleRef(ParticleOptions p) throws Exception {
            int id = -1; for (int i = 0; i < particles.size(); i++) if (particles.get(i) == p) { id = i; break; }
            if (id < 0) { id = particles.size(); particles.add(p); }
            var desc = particle(p);
            return id + ":" + desc.get("type").getAsString() + ":" + (desc.has("color") ? desc.get("color").getAsString() : "none");
        }
        String value(Object value) throws Exception {
            if (value instanceof Byte b) return "flags:" + Byte.toUnsignedInt(b);
            if (value instanceof Boolean b) return "ambient:" + b;
            if (value instanceof List<?> list) {
                var refs = new StringJoiner(","); for (var p : list) refs.add(particleRef((ParticleOptions)p));
                return "particles:[" + refs + "]";
            }
            throw new AssertionError(value);
        }
        String packed(List<SynchedEntityData.DataValue<?>> values) throws Exception {
            if (values == null) return "none";
            var result = new StringJoiner("|"); for (var v : values) result.add(v.id() + "=" + value(v.value()));
            return result.length() == 0 ? "empty" : result.toString();
        }
        void event(String name) {
            int call = calls++; out.println("event " + call + " " + name);
            if (call == op.get("fail").getAsInt()) throw new Stop();
            if (call == op.get("mutate").getAsInt()) {
                try { controls(op.getAsJsonObject("changes")); for (var action : op.getAsJsonArray("actions")) raw(action.getAsJsonObject()); }
                catch (RuntimeException e) { throw e; } catch (Exception e) { throw new AssertionError(e); }
            }
        }
        void controls(JsonObject c) throws Exception {
            if (c.has("data")) data = c.get("data").getAsInt();
            if (c.has("client")) client = c.get("client").getAsBoolean();
            if (c.has("tag")) tagField.setBoolean(actor, c.get("tag").getAsBoolean());
            if (c.has("dirty")) dirtyField.setBoolean(actor, c.get("dirty").getAsBoolean());
            dataField.set(actor, tables[data]);
        }
        void raw(JsonObject op) throws Exception {
            int target = op.get("target").getAsInt(), key = op.get("key").getAsInt(), slot = op.get("pool").getAsInt();
            switch (op.get("mode").getAsString()) {
                case "put": effects.put(keys.get(key), pool.get(slot)); break;
                case "drop": effects.remove(keys.get(key)); break;
                case "clear": effects.clear(); break;
                case "details": visibleField.setBoolean(pool.get(slot), op.get("visible").getAsBoolean()); ambientField.setBoolean(pool.get(slot), op.get("ambient").getAsBoolean()); break;
                case "controls": controls(op.getAsJsonObject("changes")); break;
                case "flags": tables[target].set(flagsKey, (byte)op.get("flags").getAsInt()); break;
                case "ambient": tables[target].set(ambientKey, op.get("ambient").getAsBoolean()); break;
                case "pack": out.println("packed " + target + " " + packed(tables[target].packDirty())); break;
                case "defaults": out.println("defaults " + target + " " + packed(tables[target].getNonDefaultValues())); break;
                default: throw new AssertionError(op);
            }
        }
        void state() throws Exception {
            out.println("state " + data + " " + client + " " + tagField.getBoolean(actor) + " " + dirtyField.getBoolean(actor) + " " + calls);
            var entries = new StringJoiner(","); for (var e : effects.entrySet()) entries.add(keys.indexOf(e.getKey()) + "=" + pool.indexOf(e.getValue()));
            out.println("effects " + entries);
            for (int i = 0; i < pool.size(); i++) out.println("instance " + i + " " + pool.get(i).isVisible() + " " + pool.get(i).isAmbient());
            for (int i = 0; i < 2; i++) {
                out.println("data " + i + " " + tables[i].isDirty());
                for (var item : items[i]) out.println("field " + item.getAccessor().id() + " " + value(item.getValue()) + " " + item.isDirty() + " " + item.isSetToDefault());
            }
        }
    }
    static final class Actor extends net.minecraft.world.entity.monster.zombie.Zombie {
        Data d; Actor() { super(null, null); }
        @Override protected void updateInvisibilityStatus() { d.event("invisibility"); super.updateInvisibilityStatus(); }
        @Override protected void removeEffectParticles() { d.event("remove-particles"); super.removeEffectParticles(); }
        @Override public boolean hasEffect(Holder<MobEffect> effect) { d.event("has " + effect.getRegisteredName()); return super.hasEffect(effect); }
        @Override public void setInvisible(boolean v) { d.event("invisible " + v); super.setInvisible(v); }
        @Override public boolean isCurrentlyGlowing() { d.event("glowing"); return super.isCurrentlyGlowing(); }
        @Override protected boolean getSharedFlag(int bit) { d.event("get-flag " + bit); return super.getSharedFlag(bit); }
        @Override protected void setSharedFlag(int bit, boolean v) { d.event("set-flag " + bit + " " + v); super.setSharedFlag(bit, v); }
        @Override public Level level() { d.event("level"); return d.level; }
        @Override public void onSyncedDataUpdated(EntityDataAccessor<?> key) { d.event("updated " + key.id()); }
    }
    static final class EffectLevel extends ServerLevel {
        Data d; EffectLevel() { super(null,null,null,null,null,null,false,0L,List.of(),false); }
        @Override public boolean isClientSide() { d.event("client"); return d.client; }
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion(); net.minecraft.server.Bootstrap.bootStrap();
        for (var effect : BuiltInRegistries.MOB_EFFECT) keys.add(BuiltInRegistries.MOB_EFFECT.wrapAsHolder(effect));
        flagsKey = (EntityDataAccessor<Byte>)field(Entity.class,"DATA_SHARED_FLAGS_ID").get(null);
        ambientKey = (EntityDataAccessor<Boolean>)field(LivingEntity.class,"DATA_EFFECT_AMBIENCE_ID").get(null);
        particlesKey = (EntityDataAccessor<List<ParticleOptions>>)field(LivingEntity.class,"DATA_EFFECT_PARTICLES").get(null);
        Path dir = Path.of(args[1]);
        if (args[0].startsWith("export-")) {
            var catalog = export(); var schema = new JsonObject(); schema.addProperty("flags",flagsKey.id()); schema.addProperty("ambient",ambientKey.id()); schema.addProperty("particles",particlesKey.id()); catalog.add("schema",schema);
            Files.writeString(dir.resolve("catalog-" + args[0].substring(7) + ".json"),catalog.toString()); return;
        }
        if (Actor.class.getSuperclass().getMethod("setInvisible",boolean.class).getDeclaringClass() != Entity.class || Actor.class.getSuperclass().getMethod("isCurrentlyGlowing").getDeclaringClass() != LivingEntity.class) throw new AssertionError("actor overrides inspected base methods");
        var u=Class.forName("sun.misc.Unsafe"); unsafe=field(u,"theUnsafe").get(null); allocate=u.getMethod("allocateInstance",Class.class);
        dataField=field(Entity.class,"entityData"); dirtyField=field(LivingEntity.class,"effectsDirty"); tagField=field(Entity.class,"hasGlowingTag"); effectsField=field(LivingEntity.class,"activeEffects"); visibleField=field(MobEffectInstance.class,"visible"); ambientField=field(MobEffectInstance.class,"ambient");
        dirtyUpdate=method("updateDirtyEffects"); particlesUpdate=method("updateSynchronizedMobEffectParticles"); glowingUpdate=method("updateGlowingStatus");
        dataCtor=SynchedEntityData.class.getDeclaredConstructor(SyncedDataHolder.class,SynchedEntityData.DataItem[].class); dataCtor.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var enriched=new PrintWriter(Files.newBufferedWriter(dir.resolve("inputs-"+args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))) {
            for(var line:(Iterable<String>)lines::iterator) {
                var f=JsonParser.parseString(line).getAsJsonObject(); var d=new Data();d.out=out;d.actor=make(Actor.class);d.actor.d=d;d.level=make(EffectLevel.class);d.level.d=d;
                effectsField.set(d.actor,d.effects);dirtyField.setBoolean(d.actor,true); d.items=(List<SynchedEntityData.DataItem<?>>[])new List<?>[2];
                for(int i=0;i<2;i++) {
                    var a=new SynchedEntityData.DataItem<Byte>(flagsKey,(byte)f.getAsJsonArray("flags").get(i).getAsInt());var b=new SynchedEntityData.DataItem<Boolean>(ambientKey,f.getAsJsonArray("ambient").get(i).getAsBoolean());var c=new SynchedEntityData.DataItem<List<ParticleOptions>>(particlesKey,List.of());
                    d.items[i]=new ArrayList<>(List.of(a,b,c));d.items[i].sort(Comparator.comparingInt(item->item.getAccessor().id()));
                    var entries=new SynchedEntityData.DataItem<?>[Math.max(flagsKey.id(),Math.max(ambientKey.id(),particlesKey.id()))+1];for(int j=0;j<entries.length;j++)entries[j]=new SynchedEntityData.DataItem<Byte>(new EntityDataAccessor<Byte>(j,EntityDataSerializers.BYTE),(byte)0);for(var item:d.items[i])entries[item.getAccessor().id()]=item;
                    d.tables[i]=dataCtor.newInstance(d.actor,entries);
                }
                for(var v:f.getAsJsonArray("pool")) {var e=v.getAsJsonObject();d.pool.add(new MobEffectInstance(keys.get(e.get("effect").getAsInt()),200,e.get("amplifier").getAsInt(),e.get("ambient").getAsBoolean(),e.get("visible").getAsBoolean(),true));}
                d.controls(f.getAsJsonObject("controls"));for(var v:f.getAsJsonArray("setup"))d.raw(v.getAsJsonObject());
                for(var v:f.getAsJsonArray("ops")) {d.op=v.getAsJsonObject();d.calls=0;String result="ok";var op=d.op;
                    try {switch(op.get("mode").getAsString()) {
                        case "dirty-update":invoke(dirtyUpdate,d.actor);break;case "invisibility":d.actor.updateInvisibilityStatus();break;case "particles":invoke(particlesUpdate,d.actor);break;case "glowing-update":invoke(glowingUpdate,d.actor);break;case "remove-particles":d.actor.removeEffectParticles();break;
                        case "glowing":result=Boolean.toString(d.actor.isCurrentlyGlowing());break;case "glow-tag":d.actor.setGlowingTag(op.get("flag").getAsBoolean());break;case "invisible":d.actor.setInvisible(op.get("flag").getAsBoolean());break;
                        case "get-flag":result=Boolean.toString(d.actor.getSharedFlag(op.get("bit").getAsInt()));break;case "set-flag":d.actor.setSharedFlag(op.get("bit").getAsInt(),op.get("flag").getAsBoolean());break;
                        case "all-ambient":result=Boolean.toString(LivingEntity.areAllEffectsAmbient(d.effects.values()));break;
                        default:d.raw(op);
                    }}catch(Stop e){result="error callback";}out.println("result "+result);d.state();
                }
                var hashes=new JsonArray();var identities=new JsonArray();for(var key:keys){hashes.add(key.hashCode());identities.add(System.identityHashCode(key));}f.add("hashes",hashes);f.add("identities",identities);enriched.println(f);
            }
            if(out.checkError()||enriched.checkError())throw new IOException("cannot write native trace");
        }
    }
}
