package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.world.item.component.Consumable;import net.minecraft.sounds.*;import net.minecraft.core.Holder;
import net.minecraft.world.entity.*;import net.minecraft.world.level.Level;import net.minecraft.server.level.ServerLevel;import net.minecraft.world.item.*;import net.minecraft.util.RandomSource;import net.minecraft.world.level.levelgen.XoroshiroRandomSource;import net.minecraft.world.phys.Vec3;import net.minecraft.core.particles.*;import net.minecraft.core.component.DataComponents;
/** Actual consumable emission composed with inherited item particles. */
public final class VanillaConsumableEmissionOracle {
 static Object unsafe;static Method allocate;static Field position,randomField;
 static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
 static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
 static int n(JsonObject v,String k){return v.get(k).getAsInt();}
 static float fl(JsonObject v,String k){return Float.intBitsToFloat((int)v.get(k).getAsLong());}
 static String bits(double v){return Long.toUnsignedString(Double.doubleToLongBits(v));}
 static int kind(Item item){return item==Items.AIR?0:item==Items.STONE?1:2;}
 static String opt(Integer value){return value==null?"none":value.toString();}
 static final class Stop extends RuntimeException{}
 static final class Data{JsonObject f;PrintWriter out;RandomSource[] random;Actor actor;ParticleLevel[] levels;ItemStack stack;ItemParticleOption first;boolean changed;int calls;
  JsonObject config(){return f.getAsJsonObject(changed?"after":"initial");}
  void reset(){try{var c=config();var p=c.getAsJsonArray("position");position.set(actor,new Vec3(p.get(0).getAsDouble(),p.get(1).getAsDouble(),p.get(2).getAsDouble()));randomField.set(actor,random[n(c,"random")]);stack.setCount(n(c,"count"));if(c.get("damage").isJsonNull())stack.remove(DataComponents.DAMAGE);else stack.set(DataComponents.DAMAGE,n(c,"damage"));if(c.get("reseed").getAsBoolean())for(int i=0;i<2;i++)random[i].setSeed(c.getAsJsonArray("seeds").get(i).getAsLong());}catch(Exception e){throw new AssertionError(e);}}
  void all(){out.println("state "+calls+" "+changed+" "+n(config(),"random")+" "+n(config(),"level")+" "+kind(stack.getItem())+" "+stack.getCount()+" "+opt(stack.get(DataComponents.DAMAGE)));}
  void event(String name){out.println("event "+name);int index=calls++;all();if(index==n(f,"fail"))throw new Stop();if(index==n(f,"mutate")){changed=true;reset();}}
 }
 static class Actor extends net.minecraft.world.entity.decoration.ArmorStand{Data d;Actor(){super(null,null);}
  @Override public void spawnItemParticles(ItemStack s,int count){if(s!=d.stack)throw new AssertionError();d.event("spawn "+count);super.spawnItemParticles(s,count);}
  @Override public void playSound(SoundEvent s,float volume,float pitch){d.event("sound "+(s==SoundEvents.GENERIC_EAT.value()?0:s==SoundEvents.PLAYER_BURP?1:-1)+" "+Integer.toUnsignedString(Float.floatToIntBits(volume))+" "+Integer.toUnsignedString(Float.floatToIntBits(pitch)));}
  @Override public RandomSource getRandom(){throw new AssertionError("virtual random getter must not be used");}
  @Override public float getXRot(){d.event("pitch");return fl(d.config(),"pitch");}
  @Override public float getYRot(){d.event("yaw");return fl(d.config(),"yaw");}
  @Override public double getEyeY(){d.event("eye");return getY()+(double)fl(d.config(),"eye");}
  @Override public Level level(){d.event("level");return d.levels[n(d.config(),"level")];}
 }
 static final class OverrideActor extends Actor implements Consumable.OverrideConsumeSound {public SoundEvent getConsumeSound(ItemStack s){if(s!=d.stack)throw new AssertionError();d.event("override");return n(d.config(),"sound")==0?SoundEvents.GENERIC_EAT.value():SoundEvents.PLAYER_BURP;}}
 static final class ParticleLevel extends ServerLevel{Data d;int id;ParticleLevel(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
  @Override public void addParticle(ParticleOptions options,double x,double y,double z,double vx,double vy,double vz){var p=(ItemParticleOption)options;if(p.getType()!=ParticleTypes.ITEM)throw new AssertionError();boolean same=d.first==null||d.first==p;if(d.first==null)d.first=p;var t=p.getItem();d.event("particle "+id+" "+kind(t.item().value())+" "+t.count()+" "+opt(t.get(DataComponents.DAMAGE))+" "+same+" "+bits(x)+" "+bits(y)+" "+bits(z)+" "+bits(vx)+" "+bits(vy)+" "+bits(vz));}
 }
 public static void main(String[] args)throws Exception{net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();for(var item:new Item[]{Items.AIR,Items.STONE,Items.BREAD})item.builtInRegistryHolder().bindComponents(net.minecraft.core.component.DataComponentMap.EMPTY);var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);position=field(Entity.class,"position");randomField=field(Entity.class,"random");Path dir=Path.of(args[1]);
 try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var d=new Data();d.f=JsonParser.parseString(line).getAsJsonObject();d.out=out;d.random=new RandomSource[2];for(int i=0;i<2;i++){long seed=d.f.getAsJsonArray("seeds").get(i).getAsLong();d.random[i]=d.f.get("xor").getAsBoolean()?new XoroshiroRandomSource(seed):RandomSource.create(seed);}d.actor=d.f.get("override").getAsBoolean()?make(OverrideActor.class):make(Actor.class);d.actor.d=d;d.levels=new ParticleLevel[2];for(int i=0;i<2;i++){d.levels[i]=make(ParticleLevel.class);d.levels[i].d=d;d.levels[i].id=i;}d.stack=new ItemStack(new Item[]{Items.AIR,Items.STONE,Items.BREAD}[n(d.f,"item")]);d.reset();d.all();var component=new Consumable(fl(d.f,"seconds"),ItemUseAnimation.values()[n(d.f,"animation")],Holder.direct(n(d.f,"sound")==0?SoundEvents.GENERIC_EAT.value():SoundEvents.PLAYER_BURP),d.f.get("has_particles").getAsBoolean(),List.of());out.println("timing "+component.consumeTicks()+" "+component.shouldEmitParticlesAndSounds(n(d.f,"remaining")));try{component.emitParticlesAndSounds(d.random[n(d.f,"emit_random")],d.actor,d.stack,n(d.f,"particles"));out.println("result ok");}catch(Stop e){out.println("result callback");}d.all();for(int i=0;i<2;i++)out.println("random "+i+" "+d.random[i].nextLong()+" "+d.random[i].nextLong());}
 }}
}
