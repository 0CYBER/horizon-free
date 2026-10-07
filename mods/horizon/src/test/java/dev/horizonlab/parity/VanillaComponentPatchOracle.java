package dev.horizonlab.parity;
import com.google.gson.*;import com.mojang.serialization.Codec;import java.io.*;import java.nio.file.*;import java.lang.reflect.*;import java.util.*;
import net.minecraft.core.component.*;
/** Actual patched maps, ordered immutable prototypes and shared copies. */
public final class VanillaComponentPatchOracle {
    static DataComponentType<Object>[] keys;static Field patchMap;static Object removed;
    static Object value(JsonElement v){if(v.isJsonNull())return null;if(v.getAsJsonPrimitive().isString())return Float.valueOf(Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16)));return Integer.valueOf(v.getAsInt());}
    static String text(Object v){if(v==null||v==removed)return "none";if(v instanceof Float f)return "f"+String.format("%08x",Float.floatToIntBits(f));return "i"+v;}
    static int id(Object key){for(int i=0;i<keys.length;i++)if(keys[i]==key)return i;throw new AssertionError();}
    static DataComponentMap prototype(JsonArray a){var b=DataComponentMap.builder();for(var ev:a){var e=ev.getAsJsonArray();b.set(keys[e.get(0).getAsInt()],value(e.get(1)));}return b.build();}
    static DataComponentPatch patch(JsonArray a){var b=DataComponentPatch.builder();for(var ev:a){var e=ev.getAsJsonArray();int k=e.get(0).getAsInt();var v=value(e.get(1));if(v==null)b.remove(keys[k]);else b.set(keys[k],v);}return b.build();}
    static String patchText(DataComponentPatch p)throws Exception{var result=new StringJoiner(",");for(var e:((Map<?,?>)patchMap.get(p)).entrySet())result.add(id(e.getKey())+"="+text(e.getValue()));return result.toString();}
    static void snapshot(PrintWriter out,PatchedDataComponentMap[] maps,DataComponentPatch[] patches)throws Exception{for(int i=0;i<maps.length;i++){var m=maps[i];var entries=new StringJoiner(",");for(var e:m)entries.add(id(e.type())+"="+text(e.value()));var ks=new StringJoiner(",");for(var k:m.keySet())ks.add(Integer.toString(id(k)));var values=new StringJoiner(",");int flags=0;for(int k=0;k<keys.length;k++){values.add(text(m.get(keys[k])));if(m.hasNonDefault(keys[k]))flags|=1<<k;}out.println("map "+i+" "+m.size()+" "+entries+" | "+ks+" | "+values+" | "+flags+" | "+patchText(m.asPatch()));}for(int i=0;i<patches.length;i++)out.println("patch "+i+" "+patchText(patches[i]));out.println("equal "+maps[0].equals(maps[1])+" "+maps[0].equals(maps[2])+" "+maps[1].equals(maps[2]));}
    @SuppressWarnings("unchecked")public static void main(String[] args)throws Exception{net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var removedField=Class.forName("net.minecraft.core.component.Removed").getDeclaredField("INSTANCE");removedField.setAccessible(true);removed=removedField.get(null);keys=new DataComponentType[16];for(int i=0;i<keys.length;i++)keys[i]=(DataComponentType<Object>)(DataComponentType<?>)DataComponentType.<Integer>builder().persistent(Codec.INT).build();patchMap=DataComponentPatch.class.getDeclaredField("map");patchMap.setAccessible(true);Path dir=Path.of(args[1]);try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var base=prototype(f.getAsJsonArray("prototype"));PatchedDataComponentMap[] maps={new PatchedDataComponentMap(base),new PatchedDataComponentMap(base),new PatchedDataComponentMap(base)};DataComponentPatch[] patches={DataComponentPatch.EMPTY,DataComponentPatch.EMPTY,DataComponentPatch.EMPTY};snapshot(out,maps,patches);for(var ov:f.getAsJsonArray("ops")){var op=ov.getAsJsonObject();int a=op.get("a").getAsInt(),b=op.get("b").getAsInt();try{switch(op.get("kind").getAsString()){
        case "set":out.println("old "+text(maps[a].set(keys[op.get("key").getAsInt()],value(op.get("value")))));break;
        case "remove":out.println("old "+text(maps[a].remove(keys[op.get("key").getAsInt()])));break;
        case "copy":maps[b]=maps[a].copy();break;
        case "save":patches[b]=maps[a].asPatch();break;
        case "apply":maps[a].applyPatch(patches[b]);break;
        case "restore":maps[a].restorePatch(patches[b]);break;
        case "clear":maps[a].clearPatch();break;
        case "build":patches[b]=patch(op.getAsJsonArray("entries"));break;
        case "from":maps[a]=PatchedDataComponentMap.fromPatch(base,patches[b]);break;
        default:throw new AssertionError();}out.println("result ok");}catch(NullPointerException e){out.println("result error");}snapshot(out,maps,patches);}}}}
}
