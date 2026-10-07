package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.component.*;import net.minecraft.world.entity.*;import net.minecraft.world.item.*;import net.minecraft.world.item.component.BlocksAttacks;
/** Native blocking eligibility and use timers, with controlled virtual item durations. */
public final class VanillaItemUseOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;static Field useItem,remaining;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static final class Data {JsonObject f;PrintWriter out;int calls;boolean changed;Actor actor;ItemStack[] items;
        JsonObject state(){return f.getAsJsonObject(changed?"after":"initial");}void reset(){try{useItem.set(actor,items[state().get("item").getAsInt()]);remaining.setInt(actor,state().get("remaining").getAsInt());}catch(Exception e){throw new AssertionError(e);}}
        void event(String s){out.println(s);int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt()){changed=true;reset();}}
        int id(ItemStack item){for(int i=0;i<items.length;i++)if(items[i]==item)return i;throw new AssertionError();}}
    static final class CustomItem extends Item {Data d;int id;CustomItem(){super(null);}@Override public int getUseDuration(ItemStack stack,LivingEntity e){d.event("duration "+id+" "+d.id(stack));return d.state().getAsJsonArray("durations").get(id).getAsInt();}}
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {Data d;Actor(){super(null,null);}@Override public boolean isUsingItem(){d.event("using");return d.state().get("using").getAsBoolean();}@Override public int getUseItemRemainingTicks(){d.event("remaining");return d.state().get("virtual_remaining").getAsInt();}@Override public ItemStack getUseItem(){throw new AssertionError("native paths use direct storage");}}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);useItem=field(LivingEntity.class,"useItem");remaining=field(LivingEntity.class,"useItemRemaining");var ctor=ItemStack.class.getDeclaredConstructor(Holder.class,int.class,PatchedDataComponentMap.class);ctor.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var d=new Data();d.f=JsonParser.parseString(line).getAsJsonObject();d.out=out;d.actor=make(Actor.class);d.actor.d=d;var a=make(CustomItem.class);a.d=d;a.id=0;var b=make(CustomItem.class);b.d=d;b.id=1;d.items=new ItemStack[]{(ItemStack)ctor.newInstance(Holder.direct(a),1,new PatchedDataComponentMap(DataComponentMap.EMPTY)),(ItemStack)ctor.newInstance(Holder.direct(d.f.get("shared_type").getAsBoolean()?a:b),1,new PatchedDataComponentMap(DataComponentMap.EMPTY)),ItemStack.EMPTY};for(int i=0;i<2;i++){var c=d.f.getAsJsonArray("components").get(i).getAsJsonObject();if(c.get("present").getAsBoolean())d.items[i].set(DataComponents.BLOCKS_ATTACKS,new BlocksAttacks(single(c.get("delay")),1,List.of(),new BlocksAttacks.ItemDamageFunction(0,0,0),Optional.empty(),Optional.empty(),Optional.empty()));}d.reset();
            try{switch(d.f.get("mode").getAsString()){case "blocking":{var item=d.actor.getItemBlockingWith();out.println("item "+(item==null?-1:d.id(item)));break;}case "is-blocking":out.println("bool "+d.actor.isBlocking());break;case "ticks":out.println("ticks "+d.actor.getTicksUsingItem());break;case "partial":out.println("float "+bits(d.actor.getTicksUsingItem(single(d.f.get("partial")))));break;default:throw new AssertionError();}out.println("result ok");}catch(Stop e){out.println("result error");}out.println("state "+d.id((ItemStack)useItem.get(d.actor))+" "+remaining.getInt(d.actor)+" "+d.calls+" "+d.changed);
        }}
    }
}
