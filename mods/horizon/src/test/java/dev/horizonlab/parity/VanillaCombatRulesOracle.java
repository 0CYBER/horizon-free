package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.nio.file.*;import java.util.*;
import net.minecraft.world.damagesource.*;import net.minecraft.world.entity.player.Player;import net.minecraft.world.item.*;import net.minecraft.world.level.Level;import net.minecraft.server.level.ServerLevel;
/** Actual CombatRules; weapon paths use empty or unenchanted native stacks. */
public final class VanillaCombatRulesOracle {
    static final class Stop extends RuntimeException {}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static final class Data {JsonObject f;PrintWriter out;int calls;boolean changed;Level level;ItemStack stack;
        void query(String text){out.println(text);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt())changed=true;}
        boolean flag(String name){return f.get((changed?"after_":"")+name).getAsBoolean();}}
    static final class Source extends DamageSource {Data d;Source(){super(null);}@Override public ItemStack getWeaponItem(){d.query("weapon");return d.flag("weapon")?d.stack:null;}}
    static final class Actor extends Player {Data d;Actor(){super(null,null);}@Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}@Override public Level level(){d.query("level");return d.flag("server")?d.level:null;}}
    static final class Server extends ServerLevel {Server(){super(null,null,null,null,null,null,false,0L,List.of(),false);}}
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);var unsafe=uf.get(null);var allocate=u.getMethod("allocateInstance",Class.class);var level=(Server)allocate.invoke(unsafe,Server.class);var stackConstructor=ItemStack.class.getDeclaredConstructor(net.minecraft.core.Holder.class,int.class,net.minecraft.core.component.PatchedDataComponentMap.class);stackConstructor.setAccessible(true);var unenchanted=(ItemStack)stackConstructor.newInstance(net.minecraft.core.registries.BuiltInRegistries.ITEM.wrapAsHolder(Items.DIAMOND_SWORD),1,new net.minecraft.core.component.PatchedDataComponentMap(net.minecraft.core.component.DataComponentMap.EMPTY));Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.level=level;d.stack=f.get("empty").getAsBoolean()?ItemStack.EMPTY:unenchanted;var source=(Source)allocate.invoke(unsafe,Source.class);source.d=d;var actor=(Actor)allocate.invoke(unsafe,Actor.class);actor.d=d;
            try{float value=f.get("mode").getAsString().equals("magic")?CombatRules.getDamageAfterMagicAbsorb(single(f.get("damage")),single(f.get("armor"))):CombatRules.getDamageAfterAbsorb(actor,single(f.get("damage")),source,single(f.get("armor")),single(f.get("toughness")));out.println("value "+bits(value));out.println("result ok");}catch(Stop e){out.println("result error");}out.println("queries "+d.calls);
        }}
    }
}
