package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.lang.invoke.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.component.*;import net.minecraft.core.registries.*;import net.minecraft.resources.*;import net.minecraft.tags.*;import net.minecraft.world.InteractionHand;import net.minecraft.world.entity.*;import net.minecraft.world.entity.player.*;import net.minecraft.world.damagesource.*;import net.minecraft.world.effect.*;import net.minecraft.world.item.*;import net.minecraft.world.item.component.*;import net.minecraft.server.level.*;import net.minecraft.server.PlayerAdvancements;import net.minecraft.sounds.*;import net.minecraft.stats.Stats;
/** Invoke the actual LivingEntity outer hurt method, retaining virtual service hooks. */
public final class VanillaLivingHurtOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;static ItemStack plain,shield;static Field stamp,lastSource,noAction,cooldown,lastHurt,duration,time;
    static <T>T make(Class<T> c)throws Exception{return c.cast(allocate.invoke(unsafe,c));}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static String state(LivingEntity a){try{return noAction.getInt(a)+" "+cooldown.getInt(a)+" "+bits(lastHurt.getFloat(a))+" "+duration.getInt(a)+" "+time.getInt(a)+" "+(lastSource.get(a)!=null)+" "+stamp.getLong(a);}catch(Exception e){throw new AssertionError(e);}}
    static final class Data {JsonObject op;PrintWriter out;int calls;boolean hit;float recorded;Server level;PlayerAdvancements advancements;List<MobEffectInstance> effects;
        boolean flag(String n){return op.get(n).getAsBoolean();}void event(String s){out.println(s);if(calls++==op.get("fail").getAsInt())throw new Stop();}}
    static final class Source extends DamageSource {Data d;Source(){super(null);}
        @Override public boolean is(TagKey<DamageType> tag){String n=tag==DamageTypeTags.IS_FIRE?"fire":tag==DamageTypeTags.IS_FREEZING?"freezing":tag==DamageTypeTags.DAMAGES_HELMET?"damages_helmet":tag==DamageTypeTags.BYPASSES_COOLDOWN?"bypasses_cooldown":tag==DamageTypeTags.NO_IMPACT?"no_impact":tag==DamageTypeTags.NO_KNOCKBACK?"no_knockback":tag==DamageTypeTags.BYPASSES_INVULNERABILITY?"bypasses_invulnerability":null;if(n==null)throw new AssertionError();d.event("tag "+n);return d.flag(n);}
        @Override public boolean is(ResourceKey<DamageType> k){if(k!=DamageTypes.THORNS)throw new AssertionError();d.event("thorns");return false;}
        @Override public Entity getEntity(){d.event("attacker");return null;}}
    static final class Server extends ServerLevel {Data d;Server(){super(null,null,null,null,null,null,false,0L,List.of(),false);}@Override public void broadcastDamageEvent(Entity a,DamageSource s){d.event("broadcast");}@Override public long getGameTime(){d.event("game-time");return d.op.get("game_time").getAsLong();}}
    static final class Effect extends MobEffectInstance {Data d;int id;Effect(){super(MobEffects.RESISTANCE);}@Override public void onMobHurt(ServerLevel l,LivingEntity a,DamageSource s,float v){d.event("effect "+id+" "+bits(v));}}
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {Actor(){super(null,null);}

        Data d;
        @Override public boolean isInvulnerableTo(ServerLevel l,DamageSource s){d.event("invulnerable");return d.flag("invulnerable");}
        @Override public boolean isDeadOrDying(){d.event("dead");return d.flag(d.hit?"dead_after":"dead_before");}
        @Override public boolean hasEffect(Holder<MobEffect> e){if(e!=MobEffects.FIRE_RESISTANCE)throw new AssertionError();d.event("fire-resistance");return d.flag("fire_resistance");}
        @Override public boolean isSleeping(){d.event("sleeping");return d.flag("sleeping");}
        @Override public void stopSleeping(){d.event("wake");}
        @Override public ItemStack getUseItem(){d.event("use-item");return d.flag("component")?shield:ItemStack.EMPTY;}
        @Override public float applyItemBlocking(ServerLevel l,DamageSource s,float v){d.event("blocking "+bits(v));return single(d.op.get("blocked"));}
        @Override public boolean is(TagKey<EntityType<?>> tag){if(tag!=EntityTypeTags.FREEZE_HURTS_EXTRA_TYPES)throw new AssertionError();d.event("freeze-extra");return d.flag("freeze_extra");}
        @Override public ItemStack getItemBySlot(EquipmentSlot slot){if(slot!=EquipmentSlot.HEAD)throw new AssertionError();d.event("helmet");return d.flag("helmet")?plain:ItemStack.EMPTY;}
        @Override protected void hurtHelmet(DamageSource s,float v){d.event("hurt-helmet "+bits(v));}
        @Override protected void actuallyHurt(ServerLevel l,DamageSource s,float v){d.event("hurt "+bits(v)+" "+state(this));d.hit=true;d.recorded=v;if(d.flag("mutate_hit")){damageCooldownTime=d.op.get("hit_cooldown").getAsInt();lastHurt=single(d.op.get("hit_last"));hurtDuration=d.op.get("hit_duration").getAsInt();hurtTime=d.op.get("hit_time").getAsInt();}}
        @Override protected void resolveMobResponsibleForDamage(DamageSource s){d.event("resolve-mob");}
        @Override protected Player resolvePlayerResponsibleForDamage(DamageSource s){d.event("resolve-player");return null;}
        @Override protected void markHurt(){d.event("mark");}
        @Override public void dealDefaultKnockback(DamageSource s,float v,boolean b){d.event("knockback "+bits(v)+" "+b);}
        @Override public ItemStack getItemInHand(InteractionHand h){d.event("hand "+h.ordinal());return ItemStack.EMPTY;}
        @Override protected SoundEvent getDeathSound(){d.event("death-sound");return null;}
        @Override public void makeSound(SoundEvent e){d.event("make-sound");}
        @Override public void die(DamageSource s){d.event("die");}
        @Override protected void playHurtSound(DamageSource s){d.event("hurt-sound");}
        @Override public ServerLevel level(){d.event("level");return d.level;}
        @Override public Collection<MobEffectInstance> getActiveEffects(){d.event("effects");return d.effects;}
    }
    static final class Human extends ServerPlayer {Human(){super(null,null,null,null);}

        Data d;
        @Override public boolean isInvulnerableTo(ServerLevel l,DamageSource s){d.event("invulnerable");return d.flag("invulnerable");}
        @Override public boolean isDeadOrDying(){d.event("dead");return d.flag(d.hit?"dead_after":"dead_before");}
        @Override public boolean hasEffect(Holder<MobEffect> e){if(e!=MobEffects.FIRE_RESISTANCE)throw new AssertionError();d.event("fire-resistance");return d.flag("fire_resistance");}
        @Override public boolean isSleeping(){d.event("sleeping");return d.flag("sleeping");}
        @Override public void stopSleeping(){d.event("wake");}
        @Override public ItemStack getUseItem(){d.event("use-item");return d.flag("component")?shield:ItemStack.EMPTY;}
        @Override public float applyItemBlocking(ServerLevel l,DamageSource s,float v){d.event("blocking "+bits(v));return single(d.op.get("blocked"));}
        @Override public boolean is(TagKey<EntityType<?>> tag){if(tag!=EntityTypeTags.FREEZE_HURTS_EXTRA_TYPES)throw new AssertionError();d.event("freeze-extra");return d.flag("freeze_extra");}
        @Override public ItemStack getItemBySlot(EquipmentSlot slot){if(slot!=EquipmentSlot.HEAD)throw new AssertionError();d.event("helmet");return d.flag("helmet")?plain:ItemStack.EMPTY;}
        @Override protected void hurtHelmet(DamageSource s,float v){d.event("hurt-helmet "+bits(v));}
        @Override protected void actuallyHurt(ServerLevel l,DamageSource s,float v){d.event("hurt "+bits(v)+" "+state(this));d.hit=true;d.recorded=v;if(d.flag("mutate_hit")){damageCooldownTime=d.op.get("hit_cooldown").getAsInt();lastHurt=single(d.op.get("hit_last"));hurtDuration=d.op.get("hit_duration").getAsInt();hurtTime=d.op.get("hit_time").getAsInt();}}
        @Override protected void resolveMobResponsibleForDamage(DamageSource s){d.event("resolve-mob");}
        @Override protected Player resolvePlayerResponsibleForDamage(DamageSource s){d.event("resolve-player");return null;}
        @Override protected void markHurt(){d.event("mark");}
        @Override public void dealDefaultKnockback(DamageSource s,float v,boolean b){d.event("knockback "+bits(v)+" "+b);}
        @Override public ItemStack getItemInHand(InteractionHand h){d.event("hand "+h.ordinal());return ItemStack.EMPTY;}
        @Override protected SoundEvent getDeathSound(){d.event("death-sound");return null;}
        @Override public void makeSound(SoundEvent e){d.event("make-sound");}
        @Override public void die(DamageSource s){d.event("die");}
        @Override protected void playHurtSound(DamageSource s){d.event("hurt-sound");}
        @Override public ServerLevel level(){d.event("level");return d.level;}
        @Override public Collection<MobEffectInstance> getActiveEffects(){d.event("effects");return d.effects;}

        @Override public PlayerAdvancements getAdvancements(){d.event("criterion");return d.advancements;}
        @Override public void awardStat(Identifier s,int v){if(s!=Stats.DAMAGE_BLOCKED_BY_SHIELD)throw new AssertionError();d.event("blocked-stat "+v);}
    }
    public static void main(String[] args)throws Throwable {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);
        noAction=field(LivingEntity.class,"noActionTime");cooldown=field(LivingEntity.class,"damageCooldownTime");lastHurt=field(LivingEntity.class,"lastHurt");duration=field(LivingEntity.class,"hurtDuration");time=field(LivingEntity.class,"hurtTime");stamp=field(LivingEntity.class,"lastDamageStamp");lastSource=field(LivingEntity.class,"lastDamageSource");
        var ctor=ItemStack.class.getDeclaredConstructor(Holder.class,int.class,PatchedDataComponentMap.class);ctor.setAccessible(true);plain=(ItemStack)ctor.newInstance(BuiltInRegistries.ITEM.wrapAsHolder(Items.STICK),1,new PatchedDataComponentMap(DataComponentMap.EMPTY));shield=(ItemStack)ctor.newInstance(BuiltInRegistries.ITEM.wrapAsHolder(Items.SHIELD),1,new PatchedDataComponentMap(DataComponentMap.EMPTY));shield.set(DataComponents.BLOCKS_ATTACKS,new BlocksAttacks(0,1,List.of(),new BlocksAttacks.ItemDamageFunction(0,0,0),Optional.empty(),Optional.empty(),Optional.empty()));
        var lookup=MethodHandles.privateLookupIn(LivingEntity.class,MethodHandles.lookup());var hurt=lookup.findSpecial(LivingEntity.class,"hurtServer",MethodType.methodType(boolean.class,ServerLevel.class,DamageSource.class,float.class),LivingEntity.class);
        Path dir=Path.of(args[1]);try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.out=out;d.level=make(Server.class);d.level.d=d;d.advancements=make(PlayerAdvancements.class);field(PlayerAdvancements.class,"activeTriggers").set(d.advancements,new HashMap<>());LivingEntity actor;if(f.get("player").getAsBoolean()){var a=make(Human.class);a.d=d;actor=a;}else{var a=make(Actor.class);a.d=d;actor=a;}var source=make(Source.class);source.d=d;
            noAction.setInt(actor,f.get("no_action").getAsInt());cooldown.setInt(actor,f.get("cooldown").getAsInt());lastHurt.setFloat(actor,single(f.get("last_hurt")));duration.setInt(actor,f.get("duration").getAsInt());time.setInt(actor,f.get("time").getAsInt());stamp.setLong(actor,f.get("stamp").getAsLong());if(f.get("source").getAsBoolean())lastSource.set(actor,source);
            for(var ov:f.getAsJsonArray("ops")){d.op=ov.getAsJsonObject();d.calls=0;d.hit=false;d.effects=new ArrayList<>();for(int i=0;i<d.op.get("effects").getAsInt();i++){var e=make(Effect.class);e.d=d;e.id=i;d.effects.add(e);}try{boolean result=(boolean)hurt.invokeExact(actor,(ServerLevel)d.level,(DamageSource)source,single(d.op.get("damage")));out.println("bool "+result);out.println("result ok");}catch(Stop e){out.println("result error");}out.println("state "+state(actor)+" "+bits(d.recorded)+" "+d.hit+" "+d.calls);}
        }}
    }
}
