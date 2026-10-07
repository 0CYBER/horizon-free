package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.world.damagesource.*;import net.minecraft.world.item.component.BlocksAttacks;
/** Actual blocking component math with native direct holder-set membership. */
public final class VanillaBlockingRulesOracle {
    static final class Stop extends RuntimeException {}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static final class Source extends DamageSource {JsonObject f;PrintWriter out;int calls;boolean changed;List<Holder<DamageType>> types;Source(){super(null);}@Override public Holder<DamageType> typeHolder(){out.println("type-holder");int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt())changed=true;return types.get(f.get(changed?"after_type":"type").getAsInt());}}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var a=Holder.direct(new DamageType("a",0));var b=Holder.direct(new DamageType("b",0));List<Holder<DamageType>> types=List.of(a,b);List<HolderSet<DamageType>> sets=List.of(HolderSet.direct(List.of(a)),HolderSet.direct(List.of(b)),HolderSet.direct(List.of(a,b)),HolderSet.direct(List.of()));var disable=BlocksAttacks.class.getDeclaredMethod("disableBlockingForTicks",float.class);disable.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var source=new Source();source.f=f;source.out=out;source.types=types;var reductions=new ArrayList<BlocksAttacks.DamageReduction>();for(var rv:f.getAsJsonArray("reductions")){var r=rv.getAsJsonObject();int set=r.get("set").getAsInt();reductions.add(new BlocksAttacks.DamageReduction(single(r.get("angle")),set==0?Optional.empty():Optional.of(sets.get(set-1)),single(r.get("base")),single(r.get("factor"))));}var item=f.getAsJsonObject("item");var itemDamage=new BlocksAttacks.ItemDamageFunction(single(item.get("threshold")),single(item.get("base")),single(item.get("factor")));var rules=new BlocksAttacks(single(f.get("delay")),single(f.get("disable_scale")),reductions,itemDamage,Optional.empty(),Optional.empty(),Optional.empty());float damage=single(f.get("damage"));double angle=number(f.get("angle"));
            try{out.println("delay "+rules.blockDelayTicks());out.println("disable "+disable.invoke(rules,single(f.get("disable_seconds"))));out.println("item "+itemDamage.apply(damage));out.println("blocked "+bits(rules.resolveBlockedDamage(source,damage,angle)));out.println("result ok");}catch(Stop e){out.println("result error");}out.println("queries "+source.calls+" "+source.changed);
        }}
    }
}
