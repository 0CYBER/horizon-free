package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.component.*;import net.minecraft.world.item.*;import net.minecraft.world.item.component.*;import org.apache.commons.lang3.math.Fraction;
/** Native checked fraction arithmetic, cached bundle weights and contained validation. */
public final class VanillaBundleWeightOracle {
 static Item[] items;static Constructor<ItemContainerContents> container;
 static String fraction(Fraction f){return f.getNumerator()+"/"+f.getDenominator();}
 static void bind(int i,JsonElement value){var b=DataComponentMap.builder();if(!value.isJsonNull())b.set(DataComponents.MAX_STACK_SIZE,value.getAsInt());items[i].builtInRegistryHolder().bindComponents(b.build());}
 static int n(JsonObject v,String key){return v.get(key).getAsInt();}
 public static void main(String[] args)throws Exception{
  net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();items=new Item[]{Items.STICK,Items.SHIELD};container=ItemContainerContents.class.getDeclaredConstructor(List.class);container.setAccessible(true);
  Path dir=Path.of(args[1]);try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();if(f.get("mode").getAsString().equals("fraction")){try{var a=Fraction.getFraction(n(f,"a"),n(f,"b"));var b=Fraction.getFraction(n(f,"c"),n(f,"d"));out.println("new "+fraction(a)+" "+fraction(b));try{out.println("add "+fraction(a.add(b)));}catch(ArithmeticException e){out.println("add error");}try{out.println("multiply "+fraction(a.multiplyBy(b)));}catch(ArithmeticException e){out.println("multiply error");}}catch(ArithmeticException e){out.println("new error");}continue;}
   for(int i=0;i<2;i++)bind(i,f.getAsJsonArray("prototypes").get(i));var bundles=new ArrayList<BundleContents>();for(var bv:f.getAsJsonArray("bundles")){var list=new ArrayList<ItemStackTemplate>();for(var iv:bv.getAsJsonArray()){var v=iv.getAsJsonObject();var p=DataComponentPatch.builder();if(!v.get("maximum").isJsonNull())p.set(DataComponents.MAX_STACK_SIZE,n(v,"maximum"));if(v.get("bees").getAsBoolean())p.set(DataComponents.BEES,new Bees(Collections.singletonList(null)));int nested=n(v,"nested");if(nested>=0)p.set(DataComponents.BUNDLE_CONTENTS,bundles.get(nested));list.add(new ItemStackTemplate(items[n(v,"kind")].builtInRegistryHolder(),n(v,"count"),p.build()));}bundles.add(new BundleContents(list));}
   for(var ov:f.getAsJsonArray("ops")){var op=ov.getAsJsonObject();switch(op.get("kind").getAsString()){
    case "prototype":bind(n(op,"index"),op.get("maximum"));out.println("prototype");break;
    case "weight":{var w=bundles.get(n(op,"index")).weight();out.println("weight "+(w.isError()?"error":fraction(w.getOrThrow())));break;}
    case "validate":{var s=new ItemStack(items[0],n(op,"count"));if(op.get("maximum").isJsonNull())s.remove(DataComponents.MAX_STACK_SIZE);else s.set(DataComponents.MAX_STACK_SIZE,n(op,"maximum"));if(op.get("damage").getAsBoolean())s.set(DataComponents.MAX_DAMAGE,0);int ci=n(op,"container"),bi=n(op,"bundle"),pi=n(op,"projectiles");if(ci>=0)s.set(DataComponents.CONTAINER,container.newInstance(bundles.get(ci).items().stream().map(Optional::of).toList()));if(bi>=0)s.set(DataComponents.BUNDLE_CONTENTS,bundles.get(bi));if(pi>=0)s.set(DataComponents.CHARGED_PROJECTILES,new ChargedProjectiles(bundles.get(pi).items()));out.println("valid "+ItemStack.validateStrict(s).isSuccess());break;}
    default:throw new AssertionError();}
   }
  }}
 }
}
