package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.*;import net.minecraft.world.entity.ai.attributes.*;import net.minecraft.server.level.*;import net.minecraft.network.syncher.*;
import net.minecraft.world.food.FoodData;import net.minecraft.world.Difficulty;import net.minecraft.world.damagesource.*;import net.minecraft.world.level.gamerules.*;
/** Actual registered attributes, synchronized health and hunger regeneration. */
public final class VanillaAttributeHealthOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;static EntityDataAccessor<Float> healthKey;static Field absorption,exhaustion,timer;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static String doubleBits(double v){return String.format("%016x",Double.doubleToLongBits(v));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static final class Data {JsonObject f;PrintWriter out;Actor actor;Level level;Rules rules;Sources sources;AttributeMap attributes;FoodData food;int calls;
        void effect(String s){out.println(s);if(calls++==f.get("fail").getAsInt())throw new Stop();}}
    static final class Rules extends GameRules {Data d;Rules(){super(net.minecraft.world.flag.FeatureFlagSet.of());}@SuppressWarnings("unchecked")@Override public <T>T get(GameRule<T> r){if(r!=GameRules.NATURAL_HEALTH_REGENERATION)throw new AssertionError();return (T)Boolean.valueOf(d.f.get("regen").getAsBoolean());}}
    static final class Sources extends DamageSources {DamageSource source;Sources(){super(null);}@Override public DamageSource starve(){return source;}}
    static final class Level extends ServerLevel {Data d;Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}@Override public Difficulty getDifficulty(){return Difficulty.values()[d.f.get("difficulty").getAsInt()];}@Override public GameRules getGameRules(){return d.rules;}}
    static final class Actor extends ServerPlayer {Data d;Actor(){super(null,null,null,null);}
        @Override public ServerLevel level(){return d.level;}
        @Override public double getAttributeValue(Holder<Attribute> a){return d.attributes.getValue(a);}
        @Override public void onSyncedDataUpdated(EntityDataAccessor<?> a){if(a!=healthKey)throw new AssertionError();d.effect("updated "+bits(getHealth()));}
        @Override protected void internalSetAbsorptionAmount(float v){try{absorption.setFloat(this,v);}catch(Exception e){throw new AssertionError(e);}}
        @Override public DamageSources damageSources(){return d.sources;}
        @Override public boolean hurtServer(ServerLevel l,DamageSource s,float v){if(l!=d.level||s!=d.sources.source)throw new AssertionError();d.effect("starve "+bits(v));return false;}
    }
    @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception {
        if(args[0].equals("export")){VanillaEntityAttributeOracle.main(args);return;}
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);unsafe=uf.get(null);allocate=u.getMethod("allocateInstance",Class.class);healthKey=(EntityDataAccessor<Float>)field(LivingEntity.class,"DATA_HEALTH_ID").get(null);
        absorption=field(LivingEntity.class,"absorptionAmount");exhaustion=field(FoodData.class,"exhaustionLevel");timer=field(FoodData.class,"tickTimer");var entityData=field(Entity.class,"entityData");var dead=field(LivingEntity.class,"dead");
        var ctor=SynchedEntityData.class.getDeclaredConstructor(SyncedDataHolder.class,SynchedEntityData.DataItem[].class);ctor.setAccessible(true);var keys=List.of(Attributes.MAX_HEALTH,Attributes.MAX_ABSORPTION);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.actor=make(Actor.class);d.actor.d=d;
            var type=BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(f.get("entity").getAsString()));d.attributes=new AttributeMap(VanillaEntityAttributeOracle.supplier(type));d.level=make(Level.class);d.level.d=d;d.rules=make(Rules.class);d.rules.d=d;d.sources=make(Sources.class);d.sources.source=make(DamageSource.class);
            var items=new SynchedEntityData.DataItem<?>[healthKey.id()+1];for(int i=0;i<items.length;i++)items[i]=new SynchedEntityData.DataItem<Float>(new EntityDataAccessor<Float>(i,EntityDataSerializers.FLOAT),0f);var health=new SynchedEntityData.DataItem<Float>(healthKey,single(f.get("health")));items[healthKey.id()]=health;var data=ctor.newInstance(d.actor,items);entityData.set(d.actor,data);dead.setBoolean(d.actor,f.get("dead").getAsBoolean());absorption.setFloat(d.actor,single(f.get("absorption")));
            d.food=new FoodData();var food=f.getAsJsonObject("food");d.food.setFoodLevel(food.get("food").getAsInt());d.food.setSaturation(single(food.get("saturation")));exhaustion.setFloat(d.food,single(food.get("exhaustion")));timer.setInt(d.food,food.get("timer").getAsInt());
            for(var ov:f.getAsJsonArray("ops")){var op=ov.getAsJsonObject();var key=keys.get(op.get("attribute").getAsInt());float v=single(op.get("value"));try{switch(op.get("mode").getAsString()){
                case "base":d.attributes.getInstance(key).setBaseValue(number(op.get("amount")));break;
                case "add":d.attributes.getInstance(key).addOrUpdateTransientModifier(new AttributeModifier(Identifier.parse(op.get("id").getAsString()),number(op.get("amount")),AttributeModifier.Operation.values()[op.get("operation").getAsInt()]));break;
                case "remove":d.attributes.getInstance(key).removeModifier(Identifier.parse(op.get("id").getAsString()));break;
                case "reset":d.attributes.resetBaseValue(key);break;
                case "set":d.actor.setHealth(v);break;case "heal":d.actor.heal(v);break;case "absorption":d.actor.setAbsorptionAmount(v);break;
                case "hurt":out.println("bool "+d.actor.isHurt());break;case "dead":out.println("bool "+d.actor.isDeadOrDying());break;
                case "tick":d.food.tick(d.actor);break;case "eat":d.food.eat(op.get("nutrition").getAsInt(),v);break;case "exhaust":d.food.addExhaustion(v);break;
                case "pack":{var p=data.packDirty();out.println("packed "+(p==null?"none":bits((Float)p.getFirst().value())));break;}
                default:throw new AssertionError();}out.println("result ok");}catch(Stop e){out.println("result error");}
                out.println("state "+bits(health.getValue())+" "+data.isDirty()+" "+health.isSetToDefault()+" "+bits(absorption.getFloat(d.actor))+" "+d.food.getFoodLevel()+" "+bits(d.food.getSaturationLevel())+" "+bits(exhaustion.getFloat(d.food))+" "+timer.getInt(d.food)+" "+d.calls);
                out.println("limits "+doubleBits(d.attributes.getValue(keys.get(0)))+" "+doubleBits(d.attributes.getValue(keys.get(1))));
            }
        }}
    }
}
