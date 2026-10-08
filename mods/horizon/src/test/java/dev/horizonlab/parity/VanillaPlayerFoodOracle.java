package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.world.food.FoodData;import net.minecraft.world.entity.player.*;import net.minecraft.world.level.Level;import net.minecraft.server.level.ServerLevel;
/** Actual player hunger guards, including direct versus virtual storage. */
public final class VanillaPlayerFoodOracle {
    static final class Stop extends RuntimeException {}
    static Field foodField,abilitiesField,exhaustion;
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static final class Data {JsonObject f;PrintWriter out;int calls;boolean changed;Actor actor;Food[] foods;Abilities direct,virtual;Level level;
        JsonObject state(){return f.getAsJsonObject(changed?"after":"initial");}
        void reset(){var s=state();direct.invulnerable=s.get("invulnerable").getAsBoolean();virtual.mayfly=s.get("mayfly").getAsBoolean();virtual.invulnerable=!direct.invulnerable;try{foodField.set(actor,foods[s.get("direct").getAsInt()]);for(int i=0;i<2;i++){var v=s.getAsJsonArray("foods").get(i).getAsJsonObject();foods[i].setFoodLevel(v.get("food").getAsInt());exhaustion.setFloat(foods[i],single(v.get("exhaustion")));}}catch(Exception e){throw new AssertionError(e);}}
        void query(String s){if(f.get("stored").getAsBoolean())return;out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt()){changed=true;reset();}}
    }
    static final class Food extends FoodData {Data d;int id;@Override public boolean hasEnoughFood(){d.query("enough "+id);return super.hasEnoughFood();}@Override public boolean needsFood(){d.query("needs "+id);return super.needsFood();}@Override public void addExhaustion(float v){d.query("exhaust "+id+" "+bits(v));super.addExhaustion(v);}}
    static final class Server extends ServerLevel {Data d;Server(){super(null,null,null,null,null,null,false,0L,List.of(),false);}@Override public boolean isClientSide(){d.query("client");return d.state().get("client").getAsBoolean();}}
    static final class Actor extends Player {Data d;Actor(){super(null,null);}@Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}@Override public Level level(){d.query("level");return d.level;}@Override public FoodData getFoodData(){d.query("food");return d.foods[d.state().get("virtual").getAsInt()];}@Override public Abilities getAbilities(){d.query("abilities");return d.virtual;}boolean enough(){return hasEnoughFoodToDoExhaustiveManoeuvres();}}
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);var unsafe=uf.get(null);var allocate=u.getMethod("allocateInstance",Class.class);foodField=field(Player.class,"foodData");abilitiesField=field(Player.class,"abilities");exhaustion=field(FoodData.class,"exhaustionLevel");Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.actor=(Actor)allocate.invoke(unsafe,Actor.class);d.actor.d=d;var level=(Server)allocate.invoke(unsafe,Server.class);level.d=d;d.level=level;d.foods=new Food[]{new Food(),new Food()};for(int i=0;i<2;i++){d.foods[i].d=d;d.foods[i].id=i;}d.direct=new Abilities();d.virtual=new Abilities();abilitiesField.set(d.actor,d.direct);d.reset();
            try{switch(f.get("mode").getAsString()){case "exhaust":d.actor.causeFoodExhaustion(single(f.get("amount")));break;case "eat":out.println("bool "+d.actor.canEat(f.get("always").getAsBoolean()));break;case "enough":out.println("bool "+d.actor.enough());break;default:throw new AssertionError();}out.println("result ok");}catch(Stop e){out.println("result error");}
            for(int i=0;i<2;i++)out.println("food "+i+" "+d.foods[i].getFoodLevel()+" "+bits(exhaustion.getFloat(d.foods[i])));out.println("queries "+d.calls);
        }}
    }
}
