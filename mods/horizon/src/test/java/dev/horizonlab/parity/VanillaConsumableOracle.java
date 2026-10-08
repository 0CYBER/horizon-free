package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.world.item.component.Consumable;import net.minecraft.sounds.*;import net.minecraft.core.Holder;
import net.minecraft.world.food.*;import net.minecraft.world.entity.player.Player;import net.minecraft.server.level.ServerPlayer;import net.minecraft.server.PlayerAdvancements;import net.minecraft.stats.*;import net.minecraft.world.level.GameType;import net.minecraft.world.level.gameevent.GameEvent;import net.minecraft.world.InteractionHand;import net.minecraft.world.InteractionResult;import net.minecraft.world.item.consume_effects.ConsumeEffect;
import net.minecraft.world.entity.*;import net.minecraft.world.level.Level;import net.minecraft.server.level.ServerLevel;import net.minecraft.world.item.*;import net.minecraft.util.RandomSource;import net.minecraft.world.level.levelgen.XoroshiroRandomSource;import net.minecraft.world.phys.Vec3;import net.minecraft.core.particles.*;import net.minecraft.core.component.DataComponents;
/** Actual consumable start/completion with real food listeners, particles and stacks. */
public final class VanillaConsumableOracle {
 static Object unsafe;static Method allocate;static Field position,randomField;
 static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
 static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
 static int n(JsonObject v,String k){return v.get(k).getAsInt();}
 static float fl(JsonObject v,String k){return Float.intBitsToFloat((int)v.get(k).getAsLong());}
 static String bits(double v){return Long.toUnsignedString(Double.doubleToLongBits(v));}
 static int kind(Item item){return item==Items.AIR?0:item==Items.STONE?1:2;}
 static String opt(Integer value){return value==null?"none":value.toString();}
 static final class Stop extends RuntimeException{}
 static final class Data{JsonObject f;PrintWriter out;RandomSource[] random;LivingEntity actor;FoodData food;PlayerAdvancements advancements;ParticleLevel[] levels;ItemStack stack;ItemParticleOption first;boolean changed;int calls;
  JsonObject config(){return f.getAsJsonObject(changed?"after":"initial");}
  void reset(){try{var c=config();var p=c.getAsJsonArray("position");position.set(actor,new Vec3(p.get(0).getAsDouble(),p.get(1).getAsDouble(),p.get(2).getAsDouble()));randomField.set(actor,random[n(c,"random")]);stack.setCount(n(c,"count"));if(c.get("damage").isJsonNull())stack.remove(DataComponents.DAMAGE);else stack.set(DataComponents.DAMAGE,n(c,"damage"));var f=c.get("food_component");if(f.isJsonNull())stack.remove(DataComponents.FOOD);else{var v=f.getAsJsonObject();stack.set(DataComponents.FOOD,new FoodProperties(n(v,"nutrition"),fl(v,"saturation"),v.get("always").getAsBoolean()));}food.setFoodLevel(n(c,"food"));food.setSaturation(fl(c,"saturation"));if(c.get("reseed").getAsBoolean())for(int i=0;i<2;i++)random[i].setSeed(c.getAsJsonArray("seeds").get(i).getAsLong());}catch(Exception e){throw new AssertionError(e);}}
  void all(){out.println("state "+calls+" "+changed+" "+n(config(),"random")+" "+n(config(),"level")+" "+kind(stack.getItem())+" "+stack.getCount()+" "+opt(stack.get(DataComponents.DAMAGE)));}
  void foodState(){out.println("food "+food.getFoodLevel()+" "+Integer.toUnsignedString(Float.floatToIntBits(food.getSaturationLevel())));}
  void event(String name){out.println("event "+name);int index=calls++;all();foodState();if(index==n(f,"fail"))throw new Stop();if(index==n(f,"mutate")){changed=true;reset();}}
 }
 static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {Data d;Actor(){super(null,null);}
  @Override public void spawnItemParticles(ItemStack s,int count){if(s!=d.stack)throw new AssertionError();d.event("spawn "+count);super.spawnItemParticles(s,count);}
  @Override public void playSound(SoundEvent s,float volume,float pitch){d.event("sound "+(s==SoundEvents.GENERIC_EAT.value()?0:s==SoundEvents.PLAYER_BURP?1:-1)+" "+Integer.toUnsignedString(Float.floatToIntBits(volume))+" "+Integer.toUnsignedString(Float.floatToIntBits(pitch)));}
  @Override public RandomSource getRandom(){d.event("random");return d.random[n(d.config(),"virtual_random")];}
  @Override public float getXRot(){d.event("pitch");return fl(d.config(),"pitch");}
  @Override public float getYRot(){d.event("yaw");return fl(d.config(),"yaw");}
  @Override public double getEyeY(){d.event("eye");return getY()+(double)fl(d.config(),"eye");}
  @Override public ParticleLevel level(){d.event("level");return d.levels[n(d.config(),"level")];}
  @Override public void startUsingItem(InteractionHand hand){d.event("start "+hand.ordinal());}
  @Override public boolean hasInfiniteMaterials(){d.event("infinite");return d.config().get("infinite").getAsBoolean();}
  @Override public void gameEvent(Holder<GameEvent> event){if(event!=GameEvent.EAT&&event!=GameEvent.DRINK)throw new AssertionError();d.event("game-event "+(event==GameEvent.DRINK));}
}
 static final class Human extends Player {Data d;Human(){super(null,null);}@Override public GameType gameMode(){return GameType.SURVIVAL;}@Override public boolean isSpectator(){return false;}@Override public boolean isCreative(){return false;}
  @Override public void spawnItemParticles(ItemStack s,int count){if(s!=d.stack)throw new AssertionError();d.event("spawn "+count);super.spawnItemParticles(s,count);}
  @Override public void playSound(SoundEvent s,float volume,float pitch){d.event("sound "+(s==SoundEvents.GENERIC_EAT.value()?0:s==SoundEvents.PLAYER_BURP?1:-1)+" "+Integer.toUnsignedString(Float.floatToIntBits(volume))+" "+Integer.toUnsignedString(Float.floatToIntBits(pitch)));}
  @Override public RandomSource getRandom(){d.event("random");return d.random[n(d.config(),"virtual_random")];}
  @Override public float getXRot(){d.event("pitch");return fl(d.config(),"pitch");}
  @Override public float getYRot(){d.event("yaw");return fl(d.config(),"yaw");}
  @Override public double getEyeY(){d.event("eye");return getY()+(double)fl(d.config(),"eye");}
  @Override public ParticleLevel level(){d.event("level");return d.levels[n(d.config(),"level")];}
  @Override public void startUsingItem(InteractionHand hand){d.event("start "+hand.ordinal());}
  @Override public boolean hasInfiniteMaterials(){d.event("infinite");return d.config().get("infinite").getAsBoolean();}
  @Override public void gameEvent(Holder<GameEvent> event){if(event!=GameEvent.EAT&&event!=GameEvent.DRINK)throw new AssertionError();d.event("game-event "+(event==GameEvent.DRINK));}

  @Override public boolean canEat(boolean always){d.event("can-eat "+always);return d.config().get("can_eat").getAsBoolean();}
  @Override public FoodData getFoodData(){d.event("food-data");return d.food;}
}
 static final class ServerActor extends ServerPlayer {Data d;ServerActor(){super(null,null,null,null);}
  @Override public void spawnItemParticles(ItemStack s,int count){if(s!=d.stack)throw new AssertionError();d.event("spawn "+count);super.spawnItemParticles(s,count);}
  @Override public void playSound(SoundEvent s,float volume,float pitch){d.event("sound "+(s==SoundEvents.GENERIC_EAT.value()?0:s==SoundEvents.PLAYER_BURP?1:-1)+" "+Integer.toUnsignedString(Float.floatToIntBits(volume))+" "+Integer.toUnsignedString(Float.floatToIntBits(pitch)));}
  @Override public RandomSource getRandom(){d.event("random");return d.random[n(d.config(),"virtual_random")];}
  @Override public float getXRot(){d.event("pitch");return fl(d.config(),"pitch");}
  @Override public float getYRot(){d.event("yaw");return fl(d.config(),"yaw");}
  @Override public double getEyeY(){d.event("eye");return getY()+(double)fl(d.config(),"eye");}
  @Override public ParticleLevel level(){d.event("level");return d.levels[n(d.config(),"level")];}
  @Override public void startUsingItem(InteractionHand hand){d.event("start "+hand.ordinal());}
  @Override public boolean hasInfiniteMaterials(){d.event("infinite");return d.config().get("infinite").getAsBoolean();}
  @Override public void gameEvent(Holder<GameEvent> event){if(event!=GameEvent.EAT&&event!=GameEvent.DRINK)throw new AssertionError();d.event("game-event "+(event==GameEvent.DRINK));}

  @Override public boolean canEat(boolean always){d.event("can-eat "+always);return d.config().get("can_eat").getAsBoolean();}
  @Override public FoodData getFoodData(){d.event("food-data");return d.food;}

  @Override public void awardStat(Stat<?> stat){if(stat.getType()!=Stats.ITEM_USED)throw new AssertionError();d.event("stat "+kind((Item)stat.getValue()));}
  @Override public PlayerAdvancements getAdvancements(){d.event("criterion");return d.advancements;}
 }
 static final class ParticleLevel extends ServerLevel{Data d;int id;ParticleLevel(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
  @Override public boolean isClientSide(){d.event("client "+id);return d.config().get("client").getAsBoolean();}
  @Override public void playSound(Entity excluded,double x,double y,double z,SoundEvent sound,SoundSource source,float volume,float pitch){if(excluded!=null)throw new AssertionError();int s=sound==SoundEvents.GENERIC_EAT.value()?0:sound==SoundEvents.PLAYER_BURP?1:-1;d.event("food-sound "+id+" "+s+" "+source.ordinal()+" "+bits(x)+" "+bits(y)+" "+bits(z)+" "+Integer.toUnsignedString(Float.floatToIntBits(volume))+" "+Integer.toUnsignedString(Float.floatToIntBits(pitch)));}
  @Override public void addParticle(ParticleOptions options,double x,double y,double z,double vx,double vy,double vz){var p=(ItemParticleOption)options;if(p.getType()!=ParticleTypes.ITEM)throw new AssertionError();boolean same=d.first==null||d.first==p;if(d.first==null)d.first=p;var t=p.getItem();d.event("particle "+id+" "+kind(t.item().value())+" "+t.count()+" "+opt(t.get(DataComponents.DAMAGE))+" "+same+" "+bits(x)+" "+bits(y)+" "+bits(z)+" "+bits(vx)+" "+bits(vy)+" "+bits(vz));}
 }
 static final class Effect implements ConsumeEffect {Data d;int id;Effect(Data d,int id){this.d=d;this.id=id;}public ConsumeEffect.Type<? extends ConsumeEffect> getType(){throw new AssertionError();}public boolean apply(Level level,ItemStack stack,LivingEntity actor){if(stack!=d.stack||actor!=d.actor)throw new AssertionError();d.event("effect "+((ParticleLevel)level).id+" "+id);return d.config().get("effect_result").getAsBoolean();}}
 public static void main(String[] args)throws Exception{net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();for(var item:new Item[]{Items.AIR,Items.STONE,Items.BREAD})item.builtInRegistryHolder().bindComponents(net.minecraft.core.component.DataComponentMap.EMPTY);var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);position=field(Entity.class,"position");randomField=field(Entity.class,"random");Path dir=Path.of(args[1]);
 try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var d=new Data();d.f=JsonParser.parseString(line).getAsJsonObject();d.out=out;d.random=new RandomSource[2];for(int i=0;i<2;i++){long seed=d.f.getAsJsonArray("seeds").get(i).getAsLong();d.random[i]=d.f.get("xor").getAsBoolean()?new XoroshiroRandomSource(seed):RandomSource.create(seed);}switch(n(d.f,"actor")){case 0->{var a=make(Actor.class);a.d=d;d.actor=a;}case 1->{var a=make(Human.class);a.d=d;d.actor=a;}case 2->{var a=make(ServerActor.class);a.d=d;d.actor=a;}default->throw new AssertionError();}d.food=new FoodData();d.advancements=make(PlayerAdvancements.class);field(PlayerAdvancements.class,"activeTriggers").set(d.advancements,new HashMap<>());d.levels=new ParticleLevel[2];for(int i=0;i<2;i++){d.levels[i]=make(ParticleLevel.class);d.levels[i].d=d;d.levels[i].id=i;}d.stack=new ItemStack(new Item[]{Items.AIR,Items.STONE,Items.BREAD}[n(d.f,"item")]);d.reset();d.all();var effects=new ArrayList<ConsumeEffect>();for(int i=0;i<n(d.f,"effects");i++)effects.add(new Effect(d,i));var component=new Consumable(fl(d.f,"seconds"),ItemUseAnimation.values()[n(d.f,"animation")],Holder.direct(n(d.f,"sound")==0?SoundEvents.GENERIC_EAT.value():SoundEvents.PLAYER_BURP),d.f.get("has_particles").getAsBoolean(),effects);d.foodState();try{switch(n(d.f,"op")){case 0->out.println("result can "+component.canConsume(d.actor,d.stack));case 1->{var r=component.startConsuming(d.actor,d.stack,InteractionHand.values()[n(d.f,"hand")]);if(r==InteractionResult.FAIL)out.println("result fail");else{var s=(InteractionResult.Success)r;if(s.shouldSwing()||!s.consumesAction())throw new AssertionError();if(s.heldItemTransformedTo()!=null&&s.heldItemTransformedTo()!=d.stack)throw new AssertionError();out.println("result consume "+(s.heldItemTransformedTo()==null?"none":"input"));}}case 2->{var r=component.onConsume(d.levels[n(d.f,"passed_level")],d.actor,d.stack);if(r!=d.stack)throw new AssertionError();out.println("result input");}default->throw new AssertionError();}}catch(Stop e){out.println("result callback");}d.all();d.foodState();for(int i=0;i<2;i++)out.println("random "+i+" "+d.random[i].nextLong()+" "+d.random[i].nextLong());}
 }}
}
