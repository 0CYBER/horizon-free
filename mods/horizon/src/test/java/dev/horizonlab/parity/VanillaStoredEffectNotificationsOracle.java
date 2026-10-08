package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.Holder;import net.minecraft.resources.Identifier;import net.minecraft.world.effect.*;import net.minecraft.world.entity.*;import net.minecraft.world.entity.ai.attributes.*;import net.minecraft.world.level.Level;import net.minecraft.server.level.ServerLevel;import com.google.common.collect.ImmutableList;
import static dev.horizonlab.parity.VanillaStoredEffectAttributesOracle.*;
/** Actual entity notifications using supplier-created instances and real dirty queues. */
public final class VanillaStoredEffectNotificationsOracle {
    static Object unsafe;static Method allocate,refresh;static Field effectDirty;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {
        Data d;EffectLevel level;Actor(){super(null,null);}
        @Override public Level level(){d.event("level");return level;}
        @Override public AttributeMap getAttributes(){d.event("attributes");return d.attributes;}
        @Override protected void onAttributeUpdated(Holder<Attribute> h){d.event("updated "+d.keyDesc(h));}
        void added(MobEffectInstance e){super.onEffectAdded(e,null);}
        void updated(MobEffectInstance e,boolean reapply){super.onEffectUpdated(e,reapply,null);}
        void removed(List<MobEffectInstance> es){super.onEffectsRemoved(es);}
    }
    static final class EffectLevel extends ServerLevel {
        Data d;EffectLevel(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public boolean isClientSide(){d.event("client");return d.op.has("client")&&d.op.get("client").getAsBoolean();}
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        effectDirty=field(LivingEntity.class,"effectsDirty");
        refresh=LivingEntity.class.getDeclaredMethod("refreshDirtyAttributes");refresh.setAccessible(true);
        var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);
        var passengers=field(Entity.class,"passengers");
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
                var actor=make(Actor.class);actor.d=d;actor.level=make(EffectLevel.class);actor.level.d=d;
                effectDirty.setBoolean(actor,true);passengers.set(actor,ImmutableList.of());
                var effect=Holder.<MobEffect>direct(d.effect);
                for(var ov:f.getAsJsonArray("ops")){d.op=ov.getAsJsonObject();d.calls=0;d.changed=false;String result="ok";
                    try{switch(d.op.get("op").getAsString()){
                        case "dirty"->effectDirty.setBoolean(actor,d.op.get("flag").getAsBoolean());
                        case "notify-add"->actor.added(new MobEffectInstance(effect,200,d.op.get("amplifier").getAsInt()));
                        case "notify-update"->actor.updated(new MobEffectInstance(effect,200,d.op.get("amplifier").getAsInt()),d.op.get("reapply").getAsBoolean());
                        case "notify-remove"->actor.removed(Collections.nCopies(d.op.get("count").getAsInt(),new MobEffectInstance(effect,200,d.op.get("amplifier").getAsInt())));
                        case "refresh"->{try{refresh.invoke(actor);}catch(InvocationTargetException e){if(e.getCause() instanceof RuntimeException x)throw x;throw new AssertionError(e.getCause());}}
                        case "get"->{var a=d.attributes.getInstance(d.key(d.op));d.track(a);result=d.reference(a);}
                        case "lookup"->result=d.reference(d.map.get(d.key(d.op)));
                        case "add"->d.effect.addAttributeModifiers(d.attributes,d.op.get("amplifier").getAsInt());
                        case "remove"->d.effect.removeAttributeModifiers(d.attributes);
                        case "modified"->{try{modified.invoke(d.attributes,d.handle(d.op.get("instance").getAsInt()));}catch(InvocationTargetException e){if(e.getCause() instanceof RuntimeException x)throw x;throw new AssertionError(e.getCause());}}
                        case "value"->result=bits(d.handle(d.op.get("instance").getAsInt()).getValue());
                        default->d.raw(d.op);
                    }}catch(Stop e){result="callback";}catch(NullPointerException e){result="null";}catch(IndexOutOfBoundsException e){result="bounds";}catch(IllegalArgumentException e){result="duplicate";}
                    out.println("result "+result);out.println("effect-dirty "+effectDirty.getBoolean(actor));d.state();
                }
                var hashes=new JsonArray();for(var a:d.instances)hashes.add(System.identityHashCode(a));f.add("instance_hashes",hashes);enriched.println(f);
            }
        }
    }
}
