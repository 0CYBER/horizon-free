package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.component.*;import net.minecraft.core.registries.*;import net.minecraft.world.item.*;import net.minecraft.server.level.*;import net.minecraft.server.PlayerAdvancements;
/** Real ItemStack durability mutations, with unenchanted component maps. */
public final class VanillaItemDurabilityOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;static Field components,count;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static String optional(Object v){return v==null?"none":v.toString();}
    static String snapshot(ItemStack s){try{var c=(PatchedDataComponentMap)components.get(s);return count.getInt(s)+" "+s.getCount()+" "+optional(c.get(DataComponents.MAX_DAMAGE))+" "+optional(c.get(DataComponents.DAMAGE))+" "+c.has(DataComponents.UNBREAKABLE)+" "+s.getMaxDamage()+" "+s.getDamageValue()+" "+s.isEmpty()+" "+s.isDamageableItem()+" "+s.isDamaged()+" "+s.isBroken()+" "+s.nextDamageWillBreak();}catch(Exception e){throw new AssertionError(e);}}
    static void reset(ItemStack s,JsonObject f){s.setCount(f.get("count").getAsInt());if(f.get("max").isJsonNull())s.remove(DataComponents.MAX_DAMAGE);else s.set(DataComponents.MAX_DAMAGE,f.get("max").getAsInt());if(f.get("damage").isJsonNull())s.remove(DataComponents.DAMAGE);else s.set(DataComponents.DAMAGE,f.get("damage").getAsInt());if(f.get("unbreakable").getAsBoolean())s.set(DataComponents.UNBREAKABLE,net.minecraft.util.Unit.INSTANCE);else s.remove(DataComponents.UNBREAKABLE);}
    static final class Data {JsonObject f;PrintWriter out;int calls;boolean changed;ItemStack stack;PlayerAdvancements advancements;
        void event(String s){out.println(s+" | "+snapshot(stack));int n=calls++;if(n==f.get("fail").getAsInt())throw new Stop();if(n==f.get("mutate_at").getAsInt()){changed=true;reset(stack,f.getAsJsonObject("after"));}}
        boolean infinite(){return f.get(changed?"after_infinite":"infinite").getAsBoolean();}}
    static final class Human extends ServerPlayer {Data d;Human(){super(null,null,null,null);}@Override public boolean hasInfiniteMaterials(){d.event("infinite");return d.infinite();}@Override public PlayerAdvancements getAdvancements(){d.event("criterion");return d.advancements;}@Override public ServerLevel level(){d.event("level");return null;}}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);components=field(ItemStack.class,"components");count=field(ItemStack.class,"count");var ctor=ItemStack.class.getDeclaredConstructor(Holder.class,int.class,PatchedDataComponentMap.class);ctor.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var d=new Data();d.f=JsonParser.parseString(line).getAsJsonObject();d.out=out;d.advancements=make(PlayerAdvancements.class);field(PlayerAdvancements.class,"activeTriggers").set(d.advancements,new HashMap<>());var actor=make(Human.class);actor.d=d;d.stack=(ItemStack)ctor.newInstance(BuiltInRegistries.ITEM.wrapAsHolder(d.f.get("air").getAsBoolean()?Items.AIR:Items.SHIELD),1,new PatchedDataComponentMap(DataComponentMap.EMPTY));reset(d.stack,d.f.getAsJsonObject("initial"));int amount=d.f.get("amount").getAsInt();
            try{switch(d.f.get("mode").getAsString()){case "hurt":d.stack.hurtAndBreak(amount,(ServerLevel)null,d.f.get("player").getAsBoolean()?actor:null,copy->{out.println("copy "+snapshot(copy));d.event("break");});break;case "without":d.stack.hurtWithoutBreaking(amount,actor);break;case "set":d.stack.setDamageValue(amount);break;case "grow":d.stack.grow(amount);break;case "shrink":d.stack.shrink(amount);break;case "query":break;default:throw new AssertionError();}out.println("result ok");}catch(Stop e){out.println("result error");}out.println("state "+snapshot(d.stack)+" "+d.calls+" "+d.changed);
        }}
    }
}
