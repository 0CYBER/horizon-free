package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.component.*;import net.minecraft.world.item.*;
/** Native template construction, patch precedence, validation and rollback. */
public final class VanillaItemTemplateOracle {
 static DataComponentType<Integer>[] keys;
 static Item[] items;static Field components,count,patchMap;static Object removed;
 static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
 static String opt(Object o){return o==null||o==removed?"none":o.toString();}
 static int kind(Item i){for(int k=0;k<items.length;k++)if(items[k]==i)return k;throw new AssertionError();}
 static int key(Object k){for(int i=0;i<keys.length;i++)if(keys[i]==k)return i;throw new AssertionError();}
 static DataComponentMap prototype(JsonArray a){var b=DataComponentMap.builder();for(var e:a){var p=e.getAsJsonArray();b.set(keys[p.get(0).getAsInt()],p.get(1).getAsInt());}return b.build();}
 static DataComponentPatch patch(JsonArray a){var b=DataComponentPatch.builder();for(var e:a){var p=e.getAsJsonArray();if(p.get(1).isJsonNull())b.remove(keys[p.get(0).getAsInt()]);else b.set(keys[p.get(0).getAsInt()],p.get(1).getAsInt());}return b.build();}
 static String patchText(DataComponentPatch p)throws Exception{var j=new StringJoiner(",");for(var e:((Map<?,?>)patchMap.get(p)).entrySet())j.add(key(e.getKey())+"="+opt(e.getValue()));return j.toString();}
 static void all(PrintWriter out,ItemStack[] stacks,ItemStackTemplate[] templates)throws Exception{
  for(int i=0;i<3;i++){var s=stacks[i];var v=new StringJoiner(",");for(var k:keys)v.add(opt(s.get(k)));out.println("stack "+i+" "+(s==ItemStack.EMPTY)+" "+kind(s.getItem())+" "+count.getInt(s)+" "+s.getCount()+" "+s.getPopTime()+" "+s.isEmpty()+" "+ItemStack.validateStrict(s).isSuccess()+" | "+v+" | "+patchText(((PatchedDataComponentMap)components.get(s)).asPatch()));}
  for(int i=0;i<3;i++){var t=templates[i];if(t==null){out.println("template "+i+" none");continue;}var v=new StringJoiner(",");for(var k:keys)v.add(opt(t.get(k)));out.println("template "+i+" "+kind(t.item().value())+" "+t.count()+" | "+v+" | "+patchText(t.components()));}
  for(int a=0;a<3;a++)for(int b=a+1;b<3;b++)out.println("same "+a+" "+b+" "+(stacks[a]==stacks[b])+" "+(templates[a]==templates[b]));
 }
 public static void main(String[] args)throws Exception{
  net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();keys=new DataComponentType[]{DataComponents.MAX_STACK_SIZE,DataComponents.MAX_DAMAGE,DataComponents.DAMAGE};items=new Item[]{Items.AIR,Items.STICK,Items.SHIELD};components=field(ItemStack.class,"components");count=field(ItemStack.class,"count");patchMap=field(DataComponentPatch.class,"map");removed=field(Class.forName("net.minecraft.core.component.Removed"),"INSTANCE").get(null);
  Path dir=Path.of(args[1]);try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();for(int i=0;i<3;i++)items[i].builtInRegistryHolder().bindComponents(prototype(f.getAsJsonArray("prototypes").get(i).getAsJsonArray()));ItemStack.EMPTY.setCount(0);ItemStack.EMPTY.setPopTime(0);((PatchedDataComponentMap)components.get(ItemStack.EMPTY)).clearPatch();var stacks=new ItemStack[3];var templates=new ItemStackTemplate[3];for(int i=0;i<3;i++){var v=f.getAsJsonArray("stacks").get(i).getAsJsonObject();int id=v.get("item").getAsInt();stacks[i]=id<0?ItemStack.EMPTY:new ItemStack(items[id],v.get("count").getAsInt());if(id>=0){stacks[i].setPopTime(5);((PatchedDataComponentMap)components.get(stacks[i])).restorePatch(patch(v.getAsJsonArray("patch")));}}all(out,stacks,templates);
   for(var ov:f.getAsJsonArray("ops")){var op=ov.getAsJsonObject();int a=op.get("a").getAsInt(),b=op.get("b").getAsInt(),n=op.get("count").getAsInt();try{switch(op.get("kind").getAsString()){
    case "new":templates[b]=new ItemStackTemplate(items[op.get("item").getAsInt()].builtInRegistryHolder(),n,patch(op.getAsJsonArray("patch")));break;
    case "from":templates[b]=ItemStackTemplate.fromStack(stacks[a]);break;
    case "from-nonempty":templates[b]=ItemStackTemplate.fromNonEmptyStack(stacks[a]);break;
    case "from-count":templates[b]=ItemStackTemplate.fromNonEmptyStack(stacks[a],n);break;
    case "with":templates[b]=templates[a].withCount(n);break;
    case "create":stacks[b]=templates[a].create();break;
    case "apply":stacks[b]=templates[a].apply(patch(op.getAsJsonArray("patch")));break;
    case "apply-count":stacks[b]=templates[a].apply(n,patch(op.getAsJsonArray("patch")));break;
    case "validate-patch":stacks[a].applyComponentsAndValidate(patch(op.getAsJsonArray("patch")));break;
    case "count":stacks[a].setCount(n);break;
    case "prototype":items[op.get("item").getAsInt()].builtInRegistryHolder().bindComponents(prototype(op.getAsJsonArray("prototype")));break;
    default:throw new AssertionError();}out.println("result ok");}catch(IllegalStateException|NullPointerException e){out.println("result error");}all(out,stacks,templates);
   }
  }}
 }
}
