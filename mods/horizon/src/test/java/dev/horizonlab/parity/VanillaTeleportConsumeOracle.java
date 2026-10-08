package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.world.item.consume_effects.TeleportRandomlyConsumeEffect;import net.minecraft.world.item.ItemStack;import net.minecraft.core.*;import net.minecraft.world.entity.*;import net.minecraft.world.level.block.Block;import net.minecraft.tags.*;import net.minecraft.world.level.gameevent.GameEvent;import net.minecraft.server.level.ServerLevel;import net.minecraft.sounds.*;import net.minecraft.util.RandomSource;import net.minecraft.world.level.levelgen.XoroshiroRandomSource;import net.minecraft.world.phys.Vec3;
/** Actual random teleport consumption with controlled destination service. */
public final class VanillaTeleportConsumeOracle {
 static Object unsafe;static Method allocate;static Field position,grace,impact;
 static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
 static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
 static int n(JsonObject v,String k){return v.get(k).getAsInt();}
 static float fl(JsonObject v,String k){return Float.intBitsToFloat((int)v.get(k).getAsLong());}
 static String bits(double v){return Long.toUnsignedString(Double.doubleToLongBits(v));}
 static final class Stop extends RuntimeException{}
 static final class Data {JsonObject f;PrintWriter out;RandomSource[] random;LivingEntity actor;boolean changed,riding;int calls,attempts;
  JsonObject config(){return f.getAsJsonObject(changed?"after":"initial");}
  void setPosition(Vec3 p){try{position.set(actor,p);}catch(Exception e){throw new AssertionError(e);}}
  Vec3 position(){try{return (Vec3)position.get(actor);}catch(Exception e){throw new AssertionError(e);}}
  void reset(){try{var c=config();var p=c.getAsJsonArray("position");setPosition(new Vec3(p.get(0).getAsDouble(),p.get(1).getAsDouble(),p.get(2).getAsDouble()));actor.fallDistance=c.get("fall").getAsDouble();grace.setInt(actor,n(c,"grace"));impact.set(actor,c.get("impact").getAsBoolean()?new Vec3(1,2,3):null);riding=c.get("riding").getAsBoolean();if(c.get("reseed").getAsBoolean())for(int i=0;i<2;i++)random[i].setSeed(c.getAsJsonArray("seeds").get(i).getAsLong());}catch(Exception e){throw new AssertionError(e);}}
  void all(){try{var p=position();out.println("state "+calls+" "+attempts+" "+changed+" "+riding+" "+bits(p.x)+" "+bits(p.y)+" "+bits(p.z)+" "+bits(actor.fallDistance)+" "+grace.getInt(actor)+" "+(impact.get(actor)!=null));}catch(Exception e){throw new AssertionError(e);}}
  void event(String name){out.println("event "+name);int i=calls++;all();if(i==n(f,"fail"))throw new Stop();if(i==n(f,"mutate")){changed=true;reset();}}
 }
 static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {Data d;Actor(){super(null,null);}

  @Override public RandomSource getRandom(){d.event("random");return d.random[n(d.config(),"random")];}
  @Override public boolean isPassenger(){d.event("passenger");return d.riding;}
  @Override public void stopRiding(){d.event("dismount");d.riding=false;}
  @Override public Vec3 position(){d.event("position");return d.position();}
  @Override public boolean randomTeleport(double x,double y,double z,boolean particles,TagKey<Block> tag){if(!particles||tag!=BlockTags.CONSUMABLE_DOES_NOT_TELEPORT_TO)throw new AssertionError();int i=d.attempts++;d.event("teleport "+bits(x)+" "+bits(y)+" "+bits(z));int at=n(d.config(),"success_at");boolean success=at>=0&&i>=at;if(d.config().get("move").getAsBoolean())d.setPosition(new Vec3(x,y,z));return success;}
  @Override public BlockPos blockPosition(){d.event("block");return BlockPos.containing(d.position());}
  @Override public void resetFallDistance(){d.event("fall");super.resetFallDistance();}
  @Override public void resetCurrentImpulseContext(){d.event("impulse");super.resetCurrentImpulseContext();}
}
 static final class FoxActor extends net.minecraft.world.entity.animal.fox.Fox {Data d;FoxActor(){super(null,null);}

  @Override public RandomSource getRandom(){d.event("random");return d.random[n(d.config(),"random")];}
  @Override public boolean isPassenger(){d.event("passenger");return d.riding;}
  @Override public void stopRiding(){d.event("dismount");d.riding=false;}
  @Override public Vec3 position(){d.event("position");return d.position();}
  @Override public boolean randomTeleport(double x,double y,double z,boolean particles,TagKey<Block> tag){if(!particles||tag!=BlockTags.CONSUMABLE_DOES_NOT_TELEPORT_TO)throw new AssertionError();int i=d.attempts++;d.event("teleport "+bits(x)+" "+bits(y)+" "+bits(z));int at=n(d.config(),"success_at");boolean success=at>=0&&i>=at;if(d.config().get("move").getAsBoolean())d.setPosition(new Vec3(x,y,z));return success;}
  @Override public BlockPos blockPosition(){d.event("block");return BlockPos.containing(d.position());}
  @Override public void resetFallDistance(){d.event("fall");super.resetFallDistance();}
  @Override public void resetCurrentImpulseContext(){d.event("impulse");super.resetCurrentImpulseContext();}
}
 static final class TeleportLevel extends ServerLevel {Data d;int id;TeleportLevel(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
  @Override public int getMinY(){d.event("min "+id);return n(d.config(),"min");}
  @Override public int getLogicalHeight(){d.event("height "+id);return n(d.config(),"height");}
  @Override public void gameEvent(Holder<GameEvent> event,Vec3 p,GameEvent.Context context){if(event!=GameEvent.TELEPORT||context.sourceEntity()!=d.actor)throw new AssertionError();d.event("game-event "+id+" "+bits(p.x)+" "+bits(p.y)+" "+bits(p.z));}
  @Override public void playSound(Entity excluded,double x,double y,double z,SoundEvent sound,SoundSource category){boolean fox=d.actor instanceof net.minecraft.world.entity.animal.fox.Fox;if(excluded!=null||sound!=(fox?SoundEvents.FOX_TELEPORT:SoundEvents.CHORUS_FRUIT_TELEPORT)||category!=(fox?SoundSource.NEUTRAL:SoundSource.PLAYERS))throw new AssertionError();d.event("sound "+id+" "+fox+" "+bits(x)+" "+bits(y)+" "+bits(z));}
  @Override public void levelEvent(int event,BlockPos p,int data){if(event!=2017)throw new AssertionError();d.event("level-event "+id+" "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+data);}
 }
 public static void main(String[] args)throws Exception{net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);position=field(Entity.class,"position");grace=field(LivingEntity.class,"currentImpulseContextResetGraceTime");impact=field(LivingEntity.class,"currentImpulseImpactPos");Path dir=Path.of(args[1]);
 try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var d=new Data();d.f=JsonParser.parseString(line).getAsJsonObject();d.out=out;d.random=new RandomSource[2];for(int i=0;i<2;i++){long seed=d.f.getAsJsonArray("seeds").get(i).getAsLong();d.random[i]=d.f.get("xor").getAsBoolean()?new XoroshiroRandomSource(seed):RandomSource.create(seed);}if(d.f.get("fox").getAsBoolean()){var a=make(FoxActor.class);a.d=d;d.actor=a;}else{var a=make(Actor.class);a.d=d;d.actor=a;}var level=make(TeleportLevel.class);level.d=d;level.id=n(d.f,"level");d.reset();d.all();var effect=new TeleportRandomlyConsumeEffect(fl(d.f,"diameter"),d.f.get("directional").getAsBoolean());try{out.println("result "+effect.apply(level,ItemStack.EMPTY,d.actor));}catch(Stop e){out.println("result callback");}d.all();for(int i=0;i<2;i++)out.println("random "+i+" "+d.random[i].nextLong()+" "+d.random[i].nextLong());}
 }}
}
