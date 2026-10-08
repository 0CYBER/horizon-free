package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.world.effect.*;import net.minecraft.core.Holder;
/** Native instance copying, shared hidden chains, update and duration behavior. */
public final class VanillaStatusEffectOracle {
 static Field duration,amplifier,ambient,visible,icon,hidden,blend,factor,previous;
 static List<Holder<MobEffect>> effects;
 static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
 static int n(JsonObject v,String k){return v.get(k).getAsInt();}
 static float fl(JsonObject v,String k){return Float.intBitsToFloat((int)v.get(k).getAsLong());}
 static long bits(float v){return Integer.toUnsignedLong(Float.floatToIntBits(v));}
 static final class Stop extends RuntimeException{}
 static int id(IdentityHashMap<MobEffectInstance,Integer> ids,MobEffectInstance e){return ids.computeIfAbsent(e,k->ids.size());}
 static void dump(PrintWriter out,MobEffectInstance[] slots,IdentityHashMap<MobEffectInstance,Integer> ids)throws Exception {
  var visit=new ArrayList<MobEffectInstance>();var seen=new IdentityHashMap<MobEffectInstance,Boolean>();
  for(int i=0;i<slots.length;i++){var e=slots[i];out.println("slot "+i+" "+id(ids,e));if(seen.put(e,true)==null)visit.add(e);}
  for(int i=0;i<visit.size();i++){var e=visit.get(i);var h=(MobEffectInstance)hidden.get(e);String hid=h==null?"none":Integer.toString(id(ids,h));if(h!=null&&seen.put(h,true)==null)visit.add(h);var b=blend.get(e);out.println("node "+id(ids,e)+" "+effects.indexOf(e.getEffect())+" "+e.getDuration()+" "+e.getAmplifier()+" "+e.isAmbient()+" "+e.isVisible()+" "+e.showIcon()+" "+hid+" "+bits(factor.getFloat(b))+" "+bits(previous.getFloat(b)));}
 }
 public static void main(String[] args)throws Exception{net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();effects=List.of(MobEffects.SPEED,MobEffects.POISON,MobEffects.NIGHT_VISION);duration=field(MobEffectInstance.class,"duration");amplifier=field(MobEffectInstance.class,"amplifier");ambient=field(MobEffectInstance.class,"ambient");visible=field(MobEffectInstance.class,"visible");icon=field(MobEffectInstance.class,"showIcon");hidden=field(MobEffectInstance.class,"hiddenEffect");blend=field(MobEffectInstance.class,"blendState");factor=field(blend.getType(),"factor");previous=field(blend.getType(),"factorPreviousFrame");Path dir=Path.of(args[1]);
 try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var input=f.getAsJsonArray("instances");var slots=new MobEffectInstance[input.size()];for(int i=0;i<slots.length;i++){var v=input.get(i).getAsJsonObject();int h=n(v,"hidden");slots[i]=new MobEffectInstance(effects.get(n(v,"effect")),n(v,"duration"),n(v,"amplifier"),v.get("ambient").getAsBoolean(),v.get("visible").getAsBoolean(),v.get("icon").getAsBoolean(),h<0?null:slots[h]);}var ids=new IdentityHashMap<MobEffectInstance,Integer>();dump(out,slots,ids);
  for(var value:f.getAsJsonArray("ops")){var op=value.getAsJsonObject();int a=n(op,"a"),b=n(op,"b");String result="ok";try{switch(op.get("op").getAsString()){
   case "alias"->slots[b]=slots[a];case "copy"->slots[b]=new MobEffectInstance(slots[a]);case "scale"->slots[b]=slots[a].withScaledDuration(fl(op,"factor"));case "update"->result=Boolean.toString(slots[a].update(slots[b]));case "eq"->result=Boolean.toString(slots[a].equals(slots[b]));case "same"->result=Boolean.toString(slots[a]==slots[b]);case "ends"->result=Boolean.toString(slots[a].endsWithin(n(op,"value")));case "infinite"->result=Boolean.toString(slots[a].isInfiniteDuration());case "copy-blend"->slots[a].copyBlendState(slots[b]);
   case "set"->{var e=slots[a];duration.setInt(e,n(op,"duration"));amplifier.setInt(e,n(op,"amplifier"));ambient.setBoolean(e,op.get("ambient").getAsBoolean());visible.setBoolean(e,op.get("visible").getAsBoolean());icon.setBoolean(e,op.get("icon").getAsBoolean());factor.setFloat(blend.get(e),fl(op,"factor"));previous.setFloat(blend.get(e),fl(op,"previous"));}
   case "map"->result=Integer.toString(slots[a].mapDuration(v->{out.println("map "+v);if(op.get("mutate").getAsBoolean())try{duration.setInt(slots[b],n(op,"value"));}catch(Exception e){throw new AssertionError(e);}if(op.get("fail").getAsBoolean())throw new Stop();return v*n(op,"value");}));default->throw new AssertionError();
  }}catch(Stop e){result="callback";}out.println("result "+result);dump(out,slots,ids);}
 }}
 }
}
