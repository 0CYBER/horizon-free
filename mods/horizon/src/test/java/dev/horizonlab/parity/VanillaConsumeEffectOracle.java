package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.world.effect.*;import net.minecraft.core.*;import net.minecraft.world.item.*;import net.minecraft.world.item.consume_effects.*;import net.minecraft.world.entity.*;import net.minecraft.world.level.Level;import net.minecraft.server.level.ServerLevel;import net.minecraft.sounds.*;import net.minecraft.util.RandomSource;import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
/** Actual apply/remove/clear/sound consume effects with concrete instance copies. */
public final class VanillaConsumeEffectOracle {
 static Object unsafe;static Method allocate;static List<Holder<MobEffect>> effects;static Field duration,amplifier,ambient,visible,icon,hidden,blend,factor,previous;
 static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
 static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
 static int n(JsonObject v,String k){return v.get(k).getAsInt();}
 static float fl(JsonObject v,String k){return Float.intBitsToFloat((int)v.get(k).getAsLong());}
 static long bits(float v){return Integer.toUnsignedLong(Float.floatToIntBits(v));}
 static String desc(MobEffectInstance e){try{var b=blend.get(e);return effects.indexOf(e.getEffect())+" "+e.getDuration()+" "+e.getAmplifier()+" "+e.isAmbient()+" "+e.isVisible()+" "+e.showIcon()+" "+(hidden.get(e)!=null)+" "+bits(factor.getFloat(b))+" "+bits(previous.getFloat(b));}catch(Exception ex){throw new AssertionError(ex);}}
 static MobEffectInstance instance(int id,JsonObject v,MobEffectInstance hidden){return new MobEffectInstance(effects.get(id),n(v,"duration"),n(v,"amplifier"),v.get("ambient").getAsBoolean(),v.get("visible").getAsBoolean(),v.get("icon").getAsBoolean(),hidden);}
 static final class Stop extends RuntimeException{}
 static final class Data {JsonObject f;PrintWriter out;RandomSource[] random;MobEffectInstance[] inputs;List<MobEffectInstance> added=new ArrayList<>();boolean changed;int calls;
  JsonObject config(){return f.getAsJsonObject(changed?"after":"initial");}
  void reset(){try{var c=config();for(int i=0;i<3;i++){var v=c.getAsJsonArray("instances").get(i).getAsJsonObject();var e=inputs[i];duration.setInt(e,n(v,"duration"));amplifier.setInt(e,n(v,"amplifier"));ambient.setBoolean(e,v.get("ambient").getAsBoolean());visible.setBoolean(e,v.get("visible").getAsBoolean());icon.setBoolean(e,v.get("icon").getAsBoolean());factor.setFloat(blend.get(e),fl(v,"factor"));previous.setFloat(blend.get(e),fl(v,"previous"));}if(c.get("reseed").getAsBoolean())for(int i=0;i<2;i++)random[i].setSeed(c.getAsJsonArray("seeds").get(i).getAsLong());}catch(Exception e){throw new AssertionError(e);}}
  void all(){out.println("state "+calls+" "+changed+" "+n(config(),"random"));for(int i=0;i<3;i++)out.println("input "+i+" "+desc(inputs[i]));for(int i=0;i<added.size();i++)out.println("added "+i+" "+desc(added.get(i)));}
  boolean event(String name){out.println("event "+name);int i=calls++;all();if(i==n(f,"fail"))throw new Stop();if(i==n(f,"mutate")){changed=true;reset();}var results=config().getAsJsonArray("returns");return results.get(i%results.size()).getAsBoolean();}
 }
 static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {Data d;Actor(){super(null,null);}
  @Override public RandomSource getRandom(){d.event("random");return d.random[n(d.config(),"random")];}
  @Override public boolean addEffect(MobEffectInstance e,Entity source){if(source!=null)throw new AssertionError();boolean fresh=true;for(var v:d.inputs)if(v==e)fresh=false;for(var v:d.added)if(v==e)fresh=false;boolean result=d.event("add "+fresh+" "+desc(e));d.added.add(e);return result;}
  @Override public boolean removeEffect(Holder<MobEffect> e){return d.event("remove "+effects.indexOf(e));}
  @Override public boolean removeAllEffects(){return d.event("clear");}
  @Override public BlockPos blockPosition(){d.event("position");var p=d.config().getAsJsonArray("position");return new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());}
  @Override public SoundSource getSoundSource(){d.event("category");return SoundSource.values()[n(d.config(),"category")];}
 }
 static final class SoundLevel extends ServerLevel {Data d;int id;SoundLevel(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
  @Override public void playSound(Entity excluded,BlockPos p,SoundEvent sound,SoundSource category,float volume,float pitch){if(excluded!=null||volume!=1||pitch!=1)throw new AssertionError();int s=sound==SoundEvents.GENERIC_EAT.value()?0:sound==SoundEvents.PLAYER_BURP?1:-1;if(s<0)throw new AssertionError();d.event("sound "+id+" "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+s+" "+category.ordinal());}
 }
 public static void main(String[] args)throws Exception{net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();effects=List.of(MobEffects.SPEED,MobEffects.POISON,MobEffects.NIGHT_VISION);var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);duration=field(MobEffectInstance.class,"duration");amplifier=field(MobEffectInstance.class,"amplifier");ambient=field(MobEffectInstance.class,"ambient");visible=field(MobEffectInstance.class,"visible");icon=field(MobEffectInstance.class,"showIcon");hidden=field(MobEffectInstance.class,"hiddenEffect");blend=field(MobEffectInstance.class,"blendState");factor=field(blend.getType(),"factor");previous=field(blend.getType(),"factorPreviousFrame");Path dir=Path.of(args[1]);
 try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var d=new Data();d.f=JsonParser.parseString(line).getAsJsonObject();d.out=out;d.random=new RandomSource[2];for(int i=0;i<2;i++){long seed=d.f.getAsJsonArray("seeds").get(i).getAsLong();d.random[i]=d.f.get("xor").getAsBoolean()?new XoroshiroRandomSource(seed):RandomSource.create(seed);}d.inputs=new MobEffectInstance[3];for(int i=0;i<3;i++){var v=d.config().getAsJsonArray("instances").get(i).getAsJsonObject();d.inputs[i]=instance(i,v,instance(i,v,null));}var actor=make(Actor.class);actor.d=d;var levels=new SoundLevel[2];for(int i=0;i<2;i++){levels[i]=make(SoundLevel.class);levels[i].d=d;levels[i].id=i;}d.reset();d.all();
 for(var value:d.f.getAsJsonArray("ops")){var op=value.getAsJsonObject();var list=op.getAsJsonArray("list");ConsumeEffect effect;switch(n(op,"kind")){case 0->{var selected=new ArrayList<MobEffectInstance>();for(var v:list)selected.add(d.inputs[v.getAsInt()]);effect=new ApplyStatusEffectsConsumeEffect(selected,fl(op,"probability"));}case 1->{var selected=new ArrayList<Holder<MobEffect>>();for(var v:list)selected.add(effects.get(v.getAsInt()));effect=new RemoveStatusEffectsConsumeEffect(HolderSet.direct(selected));}case 2->effect=ClearAllStatusEffectsConsumeEffect.INSTANCE;case 3->effect=new PlaySoundConsumeEffect(Holder.direct(n(op,"sound")==0?SoundEvents.GENERIC_EAT.value():SoundEvents.PLAYER_BURP));default->throw new AssertionError();}try{out.println("result "+effect.apply(levels[n(op,"level")],ItemStack.EMPTY,actor));}catch(Stop e){out.println("result callback");}d.all();}
 for(int i=0;i<2;i++)out.println("random "+i+" "+d.random[i].nextLong()+" "+d.random[i].nextLong());
 }}
 }
}
