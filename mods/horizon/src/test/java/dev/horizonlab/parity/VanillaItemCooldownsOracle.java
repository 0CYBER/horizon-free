package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.component.*;import net.minecraft.resources.Identifier;import net.minecraft.world.item.*;import net.minecraft.world.item.component.UseCooldown;
/** Actual cooldown storage, expiry iteration, stack grouping and use-cooldown application. */
public final class VanillaItemCooldownsOracle {
 static Field tick,map,start,end;static Item[] items;static Object unsafe;static Method allocate;
 static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
 static int n(JsonObject o,String k){return o.get(k).getAsInt();}
 static float fl(JsonObject o,String k){return Float.intBitsToFloat((int)o.get(k).getAsLong());}
 static long bits(float f){return Integer.toUnsignedLong(Float.floatToIntBits(f));}
 static final class Stop extends RuntimeException{}
 static final class Book extends ItemCooldowns{Data d;@Override protected void onCooldownStarted(Identifier id,int ticks){if(d!=null)d.event("started "+id+" "+ticks);}@Override protected void onCooldownEnded(Identifier id){if(d!=null)d.event("ended "+id);}}
 static final class Actor extends net.minecraft.world.entity.player.Player{Data d;Actor(){super(null,null);}@Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}@Override public boolean isSpectator(){return false;}@Override public boolean isCreative(){return false;}@Override public ItemCooldowns getCooldowns(){d.event("get-cooldowns");return d.book;}}
 static final class Data{Book book;PrintWriter out;Identifier[] groups;ItemStack stack;JsonObject op;int calls;
  void snapshot(){try{var entries=new StringJoiner(",");for(var e:((Map<Identifier,?>)map.get(book)).entrySet())entries.add(e.getKey()+"="+start.getInt(e.getValue())+":"+end.getInt(e.getValue()));out.println("state "+tick.getInt(book)+" "+calls+" | "+entries);out.println("stack "+stack.getCount()+" "+book.getCooldownGroup(stack));}catch(Exception e){throw new AssertionError(e);}}
  void event(String name){out.println("event "+name);int index=calls++;snapshot();if(index==n(op,"fail"))throw new Stop();if(index==n(op,"mutate")){try{switch(op.get("action").getAsString()){
   case "add":book.addCooldown(groups[n(op,"other")],n(op,"after"));break;
   case "remove":book.removeCooldown(groups[n(op,"other")]);break;
   case "clock":tick.setInt(book,n(op,"after"));break;
   case "tick":book.tick();break;
   case "stack":stack.setCount(n(op,"after"));stack.set(DataComponents.USE_COOLDOWN,new UseCooldown(1,Optional.of(groups[n(op,"other")])));break;
   case "none":break;default:throw new AssertionError();}}catch(ReflectiveOperationException e){throw new AssertionError(e);}}}
 }
 public static void main(String[] args)throws Exception{net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();items=new Item[]{Items.AIR,Items.STICK,Items.SHIELD};for(var item:items)item.builtInRegistryHolder().bindComponents(DataComponentMap.EMPTY);tick=field(ItemCooldowns.class,"tickCount");map=field(ItemCooldowns.class,"cooldowns");var ci=Class.forName("net.minecraft.world.item.ItemCooldowns$CooldownInstance");start=field(ci,"startTime");end=field(ci,"endTime");var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);
 Path dir=Path.of(args[1]);try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.out=out;d.groups=new Identifier[f.getAsJsonArray("groups").size()];for(int i=0;i<d.groups.length;i++)d.groups[i]=Identifier.parse(f.getAsJsonArray("groups").get(i).getAsString());d.book=new Book();tick.setInt(d.book,n(f,"clock"));for(var iv:f.getAsJsonArray("initial")){var a=iv.getAsJsonArray();d.book.addCooldown(d.groups[a.get(0).getAsInt()],a.get(1).getAsInt());}d.book.d=d;d.stack=new ItemStack(items[n(f,"item")],n(f,"count"));if(f.get("component").getAsBoolean())d.stack.set(DataComponents.USE_COOLDOWN,new UseCooldown(fl(f,"seconds"),n(f,"group")<0?Optional.empty():Optional.of(d.groups[n(f,"group")])));var actor=(Actor)allocate.invoke(unsafe,Actor.class);actor.d=d;d.snapshot();
 for(var ov:f.getAsJsonArray("ops")){var op=ov.getAsJsonObject();d.op=op;d.calls=0;try{switch(op.get("kind").getAsString()){
 case "add":d.book.addCooldown(d.groups[n(op,"group")],n(op,"duration"));break;
 case "add-stack":d.book.addCooldown(d.stack,n(op,"duration"));break;
 case "remove":d.book.removeCooldown(d.groups[n(op,"group")]);break;
 case "tick":d.book.tick();break;
 case "clock":tick.setInt(d.book,n(op,"duration"));break;
 case "percent":out.println("percent "+bits(d.book.getCooldownPercent(d.stack,fl(op,"partial")))+" "+d.book.isOnCooldown(d.stack));break;
 case "duration":out.println("duration "+new UseCooldown(fl(op,"seconds")).ticks());break;
 case "apply":new UseCooldown(fl(op,"seconds"),Optional.of(d.groups[n(op,"group")])).apply(d.stack,op.get("player").getAsBoolean()?actor:null);break;
 default:throw new AssertionError();}out.println("result ok");}catch(Stop e){out.println("result callback");}catch(ConcurrentModificationException e){out.println("result modified");}d.snapshot();}
 }}}
}
