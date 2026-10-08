package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.world.food.*;import net.minecraft.world.Difficulty;import net.minecraft.server.level.*;
import net.minecraft.world.damagesource.*;import net.minecraft.world.level.gamerules.*;
/** Actual FoodData arithmetic and tick with observable player/world callbacks. */
public final class VanillaFoodOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;static Field exhaustion,timer;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static Field field(String n)throws Exception{var f=FoodData.class.getDeclaredField(n);f.setAccessible(true);return f;}
    static void set(FoodData food,JsonObject v){try{food.setFoodLevel(v.get("food").getAsInt());food.setSaturation(single(v.get("saturation")));exhaustion.setFloat(food,single(v.get("exhaustion")));timer.setInt(food,v.get("timer").getAsInt());}catch(Exception e){throw new AssertionError(e);}}
    static final class Data {JsonObject f;PrintWriter out;FoodData food;int calls;boolean changed;Level level;Rules rules;Sources sources;
        void query(String s){out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt()){set(food,f.getAsJsonObject("after"));changed=true;}}
        JsonObject state(){return changed?f.getAsJsonObject("after_world"):f.getAsJsonObject("world");}
    }
    static final class Rules extends GameRules {Data d;Rules(){super(net.minecraft.world.flag.FeatureFlagSet.of());}@SuppressWarnings("unchecked")@Override public <T>T get(GameRule<T> r){if(r!=GameRules.NATURAL_HEALTH_REGENERATION)throw new AssertionError();d.query("regeneration");return (T)Boolean.valueOf(d.state().get("regen").getAsBoolean());}}
    static final class Sources extends DamageSources {Data d;DamageSource source;Sources(){super(null);}@Override public DamageSource starve(){d.query("starve-source");return source;}}
    static final class Level extends ServerLevel {Data d;Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public Difficulty getDifficulty(){d.query("difficulty");return Difficulty.values()[d.state().get("difficulty").getAsInt()];}
        @Override public GameRules getGameRules(){d.query("rules");return d.rules;}
    }
    static final class Actor extends ServerPlayer {Data d;Actor(){super(null,null,null,null);}
        @Override public ServerLevel level(){d.query("level");return d.level;}
        @Override public boolean isHurt(){d.query("is-hurt");return d.state().get("hurt").getAsBoolean();}
        @Override public float getHealth(){d.query("health");return single(d.state().get("health"));}
        @Override public void heal(float v){d.query("heal "+bits(v));}
        @Override public DamageSources damageSources(){d.query("damage-sources");return d.sources;}
        @Override public boolean hurtServer(ServerLevel l,DamageSource s,float v){if(l!=d.level||s!=d.sources.source)throw new AssertionError();d.query("hurt "+bits(v));return false;}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);unsafe=uf.get(null);allocate=u.getMethod("allocateInstance",Class.class);exhaustion=field("exhaustionLevel");timer=field("tickTimer");Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.food=new FoodData();set(d.food,f.getAsJsonObject("initial"));d.level=make(Level.class);d.level.d=d;d.rules=make(Rules.class);d.rules.d=d;d.sources=make(Sources.class);d.sources.d=d;d.sources.source=make(DamageSource.class);var actor=make(Actor.class);actor.d=d;
            try{for(var v:f.getAsJsonArray("ops")){var op=v.getAsJsonObject();switch(op.get("mode").getAsString()){
                case "eat":d.food.eat(op.get("nutrition").getAsInt(),single(op.get("amount")));break;
                case "properties":d.food.eat(new FoodProperties(op.get("nutrition").getAsInt(),single(op.get("amount")),false));break;
                case "exhaust":d.food.addExhaustion(single(op.get("amount")));break;
                case "food":d.food.setFoodLevel(op.get("nutrition").getAsInt());break;
                case "saturation":d.food.setSaturation(single(op.get("amount")));break;
                case "tick":d.food.tick(actor);break;
                default:throw new AssertionError();}
                out.println("state "+d.food.getFoodLevel()+" "+bits(d.food.getSaturationLevel())+" "+bits(exhaustion.getFloat(d.food))+" "+timer.getInt(d.food)+" "+d.food.hasEnoughFood()+" "+d.food.needsFood());
            }out.println("result ok");}catch(Stop e){out.println("result error");}
            out.println("final "+d.food.getFoodLevel()+" "+bits(d.food.getSaturationLevel())+" "+bits(exhaustion.getFloat(d.food))+" "+timer.getInt(d.food));out.println("queries "+d.calls);
        }}
    }
}
