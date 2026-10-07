package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.world.entity.*;import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.effect.*;import net.minecraft.core.Holder;
/** Native server/client ticking, hidden downgrades and blend interpolation. */
public final class VanillaStatusTickOracle {
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
 static Object unsafe;static Method allocate;static Field removed;static Data active;
 static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
 static final class Data {JsonObject f;PrintWriter out;MobEffectInstance[] slots;IdentityHashMap<MobEffectInstance,Integer> ids=new IdentityHashMap<>();LivingEntity actor;ServerLevel[] levels;int calls;boolean changed;
  JsonObject config(){return f.getAsJsonObject(changed?"after":"initial");}
  void reset(){try{var c=config();actor.tickCount=n(c,"tick");for(int i=0;i<slots.length;i++){var v=c.getAsJsonArray("instances").get(i).getAsJsonObject();var e=slots[i];duration.setInt(e,n(v,"duration"));amplifier.setInt(e,n(v,"amplifier"));ambient.setBoolean(e,v.get("ambient").getAsBoolean());visible.setBoolean(e,v.get("visible").getAsBoolean());icon.setBoolean(e,v.get("icon").getAsBoolean());factor.setFloat(blend.get(e),fl(v,"factor"));previous.setFloat(blend.get(e),fl(v,"previous"));}}catch(Exception e){throw new AssertionError(e);}}
  void all(){try{out.println("state "+calls+" "+changed+" "+actor.tickCount);dump(out,slots,ids);}catch(Exception e){throw new AssertionError(e);}}
  void event(String name){out.println("event "+name);int i=calls++;all();if(i==n(f,"fail"))throw new Stop();if(i==n(f,"mutate")){changed=true;reset();}}
 }
 static final class Effect extends MobEffect {int id;Effect(int id){super(MobEffectCategory.BENEFICIAL,0);this.id=id;}
  @Override public boolean shouldApplyEffectTickThisTick(int time,int amp){active.event("should "+id+" "+time+" "+amp);return active.config().get("should").getAsBoolean();}
  @Override public boolean applyEffectTick(ServerLevel level,LivingEntity actor,int amp){if(actor!=active.actor)throw new AssertionError();int l=level==active.levels[0]?0:level==active.levels[1]?1:-1;if(l<0)throw new AssertionError();active.event("apply "+id+" "+l+" "+amp);return active.config().get("apply").getAsBoolean();}
  @Override public int getBlendOutAdvanceTicks(){active.event("advance "+id);return n(active.config(),"advance");}
  @Override public int getBlendInDurationTicks(){active.event("blend-duration "+id+" true");return n(active.config(),"blend_in");}
  @Override public int getBlendOutDurationTicks(){active.event("blend-duration "+id+" false");return n(active.config(),"blend_out");}
 }
 public static void main(String[] args)throws Exception{net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();effects=List.of(Holder.direct(new Effect(0)),Holder.direct(new Effect(1)),Holder.direct(new Effect(2)));var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);removed=field(Entity.class,"removalReason");duration=field(MobEffectInstance.class,"duration");amplifier=field(MobEffectInstance.class,"amplifier");ambient=field(MobEffectInstance.class,"ambient");visible=field(MobEffectInstance.class,"visible");icon=field(MobEffectInstance.class,"showIcon");hidden=field(MobEffectInstance.class,"hiddenEffect");blend=field(MobEffectInstance.class,"blendState");factor=field(blend.getType(),"factor");previous=field(blend.getType(),"factorPreviousFrame");Path dir=Path.of(args[1]);
 try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var d=new Data();active=d;d.f=JsonParser.parseString(line).getAsJsonObject();d.out=out;d.actor=make(net.minecraft.world.entity.decoration.ArmorStand.class);d.levels=new ServerLevel[]{make(ServerLevel.class),make(ServerLevel.class)};var input=d.config().getAsJsonArray("instances");d.slots=new MobEffectInstance[input.size()];for(int i=0;i<d.slots.length;i++){var v=input.get(i).getAsJsonObject();int h=n(v,"hidden");d.slots[i]=new MobEffectInstance(effects.get(n(v,"effect")),n(v,"duration"),n(v,"amplifier"),v.get("ambient").getAsBoolean(),v.get("visible").getAsBoolean(),v.get("icon").getAsBoolean(),h<0?null:d.slots[h]);}d.reset();d.all();
 for(var value:d.f.getAsJsonArray("ops")){var op=value.getAsJsonObject();int a=n(op,"a"),b=n(op,"b");String result="ok";try{switch(op.get("op").getAsString()){
  case "server"->result=Boolean.toString(d.slots[a].tickServer(d.levels[n(op,"level")],d.actor,()->d.event("downgrade")));
  case "client"->d.slots[a].tickClient();case "skip"->d.slots[a].skipBlending();
  case "factor"->{removed.set(d.actor,op.get("removed").getAsBoolean()?Entity.RemovalReason.DISCARDED:null);result=Long.toString(bits(d.slots[a].getBlendFactor(d.actor,fl(op,"partial"))));}
  case "update"->result=Boolean.toString(d.slots[a].update(d.slots[b]));case "copy"->d.slots[b]=new MobEffectInstance(d.slots[a]);case "alias"->d.slots[b]=d.slots[a];default->throw new AssertionError();
 }}catch(Stop e){result="callback";}out.println("result "+result);d.all();}
 }}
 }
}
