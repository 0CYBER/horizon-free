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

/** Real AttributeMap/Supplier allocation, modifier application and dirty queues. */
public final class VanillaStoredEffectAttributesOracle {
    static Field mapField, dirty, cached;
    static Method modified;
    static Constructor<AttributeSupplier> supplierConstructor;
    static Field field(Class<?> c, String name) throws Exception { var f=c.getDeclaredField(name); f.setAccessible(true); return f; }
    static double number(JsonElement v) { return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16)); }
    static String bits(double v) { return String.format("%016x",Double.doubleToLongBits(v)); }
    static final class Stop extends RuntimeException {}
    static final class Basic extends Attribute {
        Data d; int id, hash;
        Basic(Data d,int id,int hash) { super("horizon.fixture",d.defaults[id]); this.d=d; this.id=id; this.hash=hash; }
        @Override public int hashCode() { return hash; }
        @Override public double getDefaultValue() { if(d.op!=null)d.event("default "+id); return d.defaults[id]; }
        @Override public boolean isClientSyncable() { d.event("syncable "+id); return d.syncable[id]; }
    }
    static final class Effect extends MobEffect { Effect() { super(MobEffectCategory.BENEFICIAL,0); } }
    record Template(Identifier id,double amount,AttributeModifier.Operation operation) {}
    static final class Attrs extends AttributeMap {
        Data d;
        Attrs(Data d,AttributeSupplier supplier) { super(supplier); this.d=d; }
        @Override public AttributeInstance getInstance(Holder<Attribute> h) { d.event("get "+d.keyDesc(h)); return super.getInstance(h); }
    }
    static final class Data {
        List<Holder<Attribute>> keys=new ArrayList<>(); List<Template> templates=new ArrayList<>();
        List<AttributeInstance> instances=new ArrayList<>(); List<AttributeModifier> modifiers=new ArrayList<>();
        IdentityHashMap<AttributeInstance,Integer> ids=new IdentityHashMap<>();
        double[] defaults; boolean[] syncable; Attrs attributes; Map<Holder<Attribute>,AttributeInstance> map;
        Effect effect=new Effect(); JsonObject op; int calls; boolean changed; PrintWriter out;
        Holder<Attribute> key(JsonObject op) { int id=op.get("key").getAsInt(); return id<0?null:keys.get(id); }
        String keyDesc(Holder<Attribute> k) { return k==null?"null":Integer.toString(keys.indexOf(k)); }
        AttributeInstance handle(int id) { return id<0?null:instances.get(id); }
        void track(AttributeInstance a) { if(a!=null&&!ids.containsKey(a)){ids.put(a,instances.size());instances.add(a);} }
        String reference(AttributeInstance a) { return a==null?"null":Integer.toString(ids.get(a)); }
        String refs(Collection<AttributeModifier> mods) {
            var result=new StringJoiner(",");
            for(var m:mods){int id=-1;for(int i=0;i<modifiers.size();i++)if(modifiers.get(i)==m){id=i;break;}if(id<0){id=modifiers.size();modifiers.add(m);}
                result.add(id+":"+m.id()+":"+bits(m.amount())+":"+m.operation().ordinal());}
            return result.toString();
        }
        void state() {
            map.values().forEach(this::track); attributes.getAttributesToUpdate().forEach(this::track); attributes.getAttributesToSync().forEach(this::track);
            out.println("state "+calls+" "+changed);
            var entries=new StringJoiner(",");map.forEach((k,a)->entries.add(keyDesc(k)+"="+reference(a)));out.println("map "+entries);
            var updates=new StringJoiner(",");attributes.getAttributesToUpdate().forEach(a->updates.add(reference(a)));out.println("updates "+updates);
            var sync=new StringJoiner(",");attributes.getAttributesToSync().forEach(a->sync.add(reference(a)));out.println("sync "+sync);
            try{for(int id=0;id<instances.size();id++){var a=instances.get(id);out.println("instance "+id+" "+keyDesc(a.getAttribute())+" "+bits(a.getBaseValue())+" "+dirty.getBoolean(a)+" "+bits(cached.getDouble(a))+" | "+refs(a.getModifiers())+" | "+refs(a.getPermanentModifiers()));}}
            catch(IllegalAccessException e){throw new AssertionError(e);}
        }
        void raw(JsonObject op) {
            switch(op.get("op").getAsString()) {
                case "put"->map.put(key(op),handle(op.get("instance").getAsInt()));
                case "drop"->map.remove(key(op)); case "clear"->map.clear();
                case "queue-add","queue-drop","queue-clear"->{var h=handle(op.get("instance").getAsInt());var set=op.get("target").getAsInt()==0?attributes.getAttributesToUpdate():attributes.getAttributesToSync();switch(op.get("op").getAsString()){case "queue-add"->set.add(h);case "queue-drop"->set.remove(h);default->set.clear();}}
                case "syncable"->syncable[op.get("key").getAsInt()]=op.get("flag").getAsBoolean();
                case "default"->defaults[op.get("key").getAsInt()]=number(op.get("value"));
                default->throw new AssertionError();
            }
        }
        void event(String name) {
            int call=calls++;out.println("event "+call+" "+name);state();
            if(call==op.get("fail").getAsInt())throw new Stop();
            if(call==op.get("mutate").getAsInt()){changed=true;for(var a:op.getAsJsonArray("mutations"))raw(a.getAsJsonObject());}
        }
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        dirty=field(AttributeInstance.class,"dirty");cached=field(AttributeInstance.class,"cachedValue");mapField=field(AttributeMap.class,"attributes");
        modified=AttributeMap.class.getDeclaredMethod("onAttributeModified",AttributeInstance.class);modified.setAccessible(true);
        supplierConstructor=AttributeSupplier.class.getDeclaredConstructor(Map.class);supplierConstructor.setAccessible(true);
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));
            var enriched=new PrintWriter(Files.newBufferedWriter(dir.resolve("inputs-"+args[0]+".txt")));
            var lines=Files.lines(dir.resolve("inputs.txt"))) {
            for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.out=out;int n=f.getAsJsonArray("hashes").size();
                d.defaults=new double[n];d.syncable=new boolean[n];for(int i=0;i<n;i++){d.defaults[i]=number(f.getAsJsonArray("defaults").get(i));d.syncable[i]=f.getAsJsonArray("syncable").get(i).getAsBoolean();}
                int componentsHash=Holder.direct(new Basic(d,0,0)).hashCode();
                for(int i=0;i<n;i++){int hash=f.getAsJsonArray("hashes").get(i).getAsInt();var key=Holder.<Attribute>direct(new Basic(d,i,(hash-componentsHash)*0xbdef7bdf));if(key.hashCode()!=hash)throw new AssertionError("holder hash");d.keys.add(key);}
                var prototypes=new HashMap<Holder<Attribute>,AttributeInstance>();
                for(int i=0;i<n;i++){var base=f.getAsJsonArray("bases").get(i);if(!base.isJsonNull()){var a=new AttributeInstance(d.keys.get(i),ignored->{});a.setBaseValue(number(base));prototypes.put(d.keys.get(i),a);}}
                d.attributes=new Attrs(d,supplierConstructor.newInstance(prototypes));d.map=(Map<Holder<Attribute>,AttributeInstance>)mapField.get(d.attributes);
                for(var tv:f.getAsJsonArray("templates")){var t=tv.getAsJsonObject();d.templates.add(new Template(Identifier.parse(t.get("id").getAsString()),number(t.get("amount")),AttributeModifier.Operation.values()[t.get("operation").getAsInt()]));}
                for(var ov:f.getAsJsonArray("definitions")){var op=ov.getAsJsonObject();var t=d.templates.get(op.get("template").getAsInt());d.effect.addAttributeModifier(d.key(op),t.id(),t.amount(),t.operation());}
                for(var ov:f.getAsJsonArray("ops")){d.op=ov.getAsJsonObject();d.calls=0;d.changed=false;String result="ok";
                    try{switch(d.op.get("op").getAsString()){
                        case "get"->{var a=d.attributes.getInstance(d.key(d.op));d.track(a);result=d.reference(a);}
                        case "lookup"->result=d.reference(d.map.get(d.key(d.op)));
                        case "add"->d.effect.addAttributeModifiers(d.attributes,d.op.get("amplifier").getAsInt());
                        case "remove"->d.effect.removeAttributeModifiers(d.attributes);
                        case "modified"->{try{modified.invoke(d.attributes,d.handle(d.op.get("instance").getAsInt()));}catch(InvocationTargetException e){if(e.getCause() instanceof RuntimeException x)throw x;throw new AssertionError(e.getCause());}}
                        case "value"->result=bits(d.handle(d.op.get("instance").getAsInt()).getValue());
                        default->d.raw(d.op);
                    }}catch(Stop e){result="callback";}catch(NullPointerException e){result="null";}catch(IndexOutOfBoundsException e){result="bounds";}catch(IllegalArgumentException e){result="duplicate";}
                    out.println("result "+result);d.state();
                }
                var hashes=new JsonArray();for(var a:d.instances)hashes.add(System.identityHashCode(a));f.add("instance_hashes",hashes);enriched.println(f);
            }
        }
    }
}
