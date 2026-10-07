package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;import net.minecraft.resources.Identifier;import net.minecraft.world.entity.*;import net.minecraft.world.entity.player.Player;import net.minecraft.world.entity.ai.attributes.*;import net.minecraft.network.syncher.*;
/** Actual player health/absorption on one native synchronized table. */
public final class VanillaPlayerHealthOracle {
    static final class Stop extends RuntimeException {}
    static EntityDataAccessor<Float> healthKey,absorptionKey;
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static String values(List<SynchedEntityData.DataValue<?>> values){if(values==null)return "none";var s=new StringJoiner(",");for(var v:values)s.add(v.id()+":"+bits((Float)v.value()));return s.length()==0?"empty":s.toString();}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static final class Data {JsonObject op;PrintWriter out;AttributeMap attributes;SynchedEntityData data;int calls;
        void event(String s){out.println(s);if(calls++==op.get("fail").getAsInt())throw new Stop();}}
    static final class Actor extends Player {Data d;Actor(){super(null,null);}@Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}
        @Override public double getAttributeValue(Holder<Attribute> a){return d.attributes.getValue(a);}
        @Override public void onSyncedDataUpdated(EntityDataAccessor<?> a){var key=a.id()==healthKey.id()?healthKey:absorptionKey;d.event("updated "+key.id()+" "+bits(d.data.get(key))+" "+d.data.isDirty());if(d.op.get("pack_callback").getAsBoolean())d.out.println("callback-pack "+values(d.data.packDirty()));}
        @Override public void onSyncedDataUpdated(List<SynchedEntityData.DataValue<?>> v){d.event("assigned "+values(v));}
    }
    @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception {
        Path dir=Path.of(args[1]);if(args[0].equals("export"))VanillaEntityAttributeOracle.main(args);else{net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();}
        healthKey=(EntityDataAccessor<Float>)field(LivingEntity.class,"DATA_HEALTH_ID").get(null);absorptionKey=(EntityDataAccessor<Float>)field(Player.class,"DATA_PLAYER_ABSORPTION_ID").get(null);if(args[0].equals("export")){var ids=new JsonObject();ids.addProperty("health",healthKey.id());ids.addProperty("absorption",absorptionKey.id());Files.writeString(dir.resolve("ids.json"),ids.toString());return;}
        var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);var unsafe=uf.get(null);var allocate=u.getMethod("allocateInstance",Class.class);var ctor=SynchedEntityData.class.getDeclaredConstructor(SyncedDataHolder.class,SynchedEntityData.DataItem[].class);ctor.setAccessible(true);var storage=field(Entity.class,"entityData");var dead=field(LivingEntity.class,"dead");var keys=List.of(Attributes.MAX_HEALTH,Attributes.MAX_ABSORPTION);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.out=out;d.attributes=new AttributeMap(VanillaEntityAttributeOracle.supplier(BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse("minecraft:player"))));var actor=(Actor)allocate.invoke(unsafe,Actor.class);actor.d=d;
            var items=new SynchedEntityData.DataItem<?>[Math.max(healthKey.id(),absorptionKey.id())+1];for(int i=0;i<items.length;i++)items[i]=new SynchedEntityData.DataItem<Float>(new EntityDataAccessor<Float>(i,EntityDataSerializers.FLOAT),0f);var health=new SynchedEntityData.DataItem<Float>(healthKey,single(f.get("health")));var absorption=new SynchedEntityData.DataItem<Float>(absorptionKey,single(f.get("absorption")));items[healthKey.id()]=health;items[absorptionKey.id()]=absorption;d.data=ctor.newInstance(actor,items);storage.set(actor,d.data);dead.setBoolean(actor,f.get("dead").getAsBoolean());
            for(var ov:f.getAsJsonArray("ops")){var op=ov.getAsJsonObject();d.op=op;d.calls=0;var key=keys.get(op.get("attribute").getAsInt());float v=single(op.get("value"));try{switch(op.get("mode").getAsString()){
                case "set":actor.setHealth(v);break;case "heal":actor.heal(v);break;case "absorption":actor.setAbsorptionAmount(v);break;case "get-absorption":out.println("float "+bits(actor.getAbsorptionAmount()));break;
                case "hurt":out.println("bool "+actor.isHurt());break;case "dead":out.println("bool "+actor.isDeadOrDying());break;
                case "base":d.attributes.getInstance(key).setBaseValue(number(op.get("amount")));break;
                case "add":d.attributes.getInstance(key).addOrUpdateTransientModifier(new AttributeModifier(Identifier.parse(op.get("id").getAsString()),number(op.get("amount")),AttributeModifier.Operation.values()[op.get("operation").getAsInt()]));break;
                case "remove":d.attributes.getInstance(key).removeModifier(Identifier.parse(op.get("id").getAsString()));break;case "reset":d.attributes.resetBaseValue(key);break;
                case "pack":out.println("packed "+values(d.data.packDirty()));break;case "defaults":out.println("defaults "+values(d.data.getNonDefaultValues()));break;
                case "assign":{var data=new ArrayList<SynchedEntityData.DataValue<?>>();for(var x:op.getAsJsonArray("values")){var e=x.getAsJsonObject();data.add(new SynchedEntityData.DataValue<Float>(e.get("field").getAsInt()==0?healthKey.id():absorptionKey.id(),EntityDataSerializers.FLOAT,single(e.get("value"))));}d.data.assignValues(data);break;}
                default:throw new AssertionError();}out.println("result ok");}catch(Stop e){out.println("result error");}
                out.println("state "+bits(health.getValue())+" "+bits(absorption.getValue())+" "+d.data.isDirty()+" "+health.isDirty()+" "+absorption.isDirty()+" "+health.isSetToDefault()+" "+absorption.isSetToDefault()+" "+d.calls);
            }
        }}
    }
}
