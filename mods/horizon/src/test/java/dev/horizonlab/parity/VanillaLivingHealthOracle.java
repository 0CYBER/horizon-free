package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;
import net.minecraft.core.Holder;import net.minecraft.world.entity.*;import net.minecraft.world.entity.player.Player;import net.minecraft.world.entity.ai.attributes.*;import net.minecraft.network.syncher.*;
/** Native health methods and Float synchronized-data callback semantics. */
public final class VanillaLivingHealthOracle {
    static final class Stop extends RuntimeException {}
    static Field entityData,dead,absorption;static EntityDataAccessor<Float> healthKey;
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static final class Data {JsonObject f;PrintWriter out;int calls,active;boolean changed;Actor actor;SynchedEntityData[] stores;SynchedEntityData.DataItem<Float>[] items;
        JsonObject world(){return f.getAsJsonObject(changed?"after":"world");}
        void query(String text){out.println(text);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt()){changed=true;active=1;try{entityData.set(actor,stores[active]);dead.setBoolean(actor,world().get("dead").getAsBoolean());}catch(Exception e){throw new AssertionError(e);}}}
    }
    static final class Actor extends Player {Data d;Actor(){super(null,null);}
        @Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}
        @Override public float getHealth(){d.query("health");return super.getHealth();}
        @Override public double getAttributeValue(Holder<Attribute> a){String name=a==Attributes.MAX_HEALTH?"max-health":a==Attributes.MAX_ABSORPTION?"max-absorption":"unknown";d.query(name);return number(d.world().get(name));}
        @Override public void setHealth(float v){d.query("set "+bits(v));super.setHealth(v);}
        void base(float v){super.setHealth(v);}
        @Override protected void internalSetAbsorptionAmount(float v){d.query("absorption "+bits(v));try{absorption.setFloat(this,v);}catch(IllegalAccessException e){throw new AssertionError(e);}}
        @Override public void onSyncedDataUpdated(EntityDataAccessor<?> key){if(key!=healthKey)throw new AssertionError();d.query("updated");}
    }
    @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);var unsafe=uf.get(null);var allocate=u.getMethod("allocateInstance",Class.class);
        entityData=field(Entity.class,"entityData");dead=field(LivingEntity.class,"dead");healthKey=(EntityDataAccessor<Float>)field(LivingEntity.class,"DATA_HEALTH_ID").get(null);absorption=field(LivingEntity.class,"absorptionAmount");
        var ctor=SynchedEntityData.class.getDeclaredConstructor(SyncedDataHolder.class,SynchedEntityData.DataItem[].class);ctor.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.actor=(Actor)allocate.invoke(unsafe,Actor.class);d.actor.d=d;d.stores=new SynchedEntityData[2];d.items=(SynchedEntityData.DataItem<Float>[])new SynchedEntityData.DataItem<?>[2];
            for(int s=0;s<2;s++){var items=new SynchedEntityData.DataItem<?>[healthKey.id()+1];for(int i=0;i<items.length;i++)items[i]=new SynchedEntityData.DataItem<Float>(new EntityDataAccessor<Float>(i,EntityDataSerializers.FLOAT),0f);d.items[s]=new SynchedEntityData.DataItem<Float>(healthKey,single(f.getAsJsonArray("health").get(s)));items[healthKey.id()]=d.items[s];d.stores[s]=ctor.newInstance(d.actor,items);}
            entityData.set(d.actor,d.stores[0]);dead.setBoolean(d.actor,d.world().get("dead").getAsBoolean());absorption.setFloat(d.actor,single(f.get("absorption")));
            for(var ov:f.getAsJsonArray("ops")){var op=ov.getAsJsonObject();float v=single(op.get("value"));try{switch(op.get("mode").getAsString()){
                case "heal":d.actor.heal(v);break;case "set":d.actor.setHealth(v);break;case "base":d.actor.base(v);break;case "absorption":d.actor.setAbsorptionAmount(v);break;
                case "hurt":out.println("bool "+d.actor.isHurt());break;case "dead":out.println("bool "+d.actor.isDeadOrDying());break;
                case "sync":d.stores[d.active].set(healthKey,v,op.get("force").getAsBoolean());break;
                case "pack":{var packed=d.stores[d.active].packDirty();out.println("packed "+(packed==null?"none":bits((Float)packed.getFirst().value())));break;}
                default:throw new AssertionError();}out.println("result ok");}catch(Stop e){out.println("result error");}
                for(int s=0;s<2;s++)out.println("data "+s+" "+bits(d.items[s].getValue())+" "+d.stores[s].isDirty()+" "+d.items[s].isDirty()+" "+d.items[s].isSetToDefault());
                out.println("state "+d.active+" "+bits(absorption.getFloat(d.actor))+" "+dead.getBoolean(d.actor)+" "+d.calls);
            }
        }}
    }
}
