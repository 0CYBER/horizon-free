package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.tags.*;import net.minecraft.world.entity.*;import net.minecraft.world.entity.ai.attributes.*;import net.minecraft.world.effect.*;import net.minecraft.world.item.*;import net.minecraft.world.level.block.*;import net.minecraft.world.level.block.state.*;import net.minecraft.world.phys.Vec3;
/** Actual native private air travel, relative movement and climbing helpers. */
public final class VanillaLivingAirOracle {
    static ItemStack BOOTS;static Field amplifier;
    static MobEffectInstance effect(Data d,Holder<MobEffect> type){if(d.f.get("levitation").isJsonNull())return null;var e=new MobEffectInstance(type,100,0);try{amplifier.setInt(e,d.f.get("levitation").getAsInt());}catch(ReflectiveOperationException ex){throw new AssertionError(ex);}return e;}
    static final class Stop extends RuntimeException {}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static Vec3 vec(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static String vector(Vec3 v){return String.format("%016x %016x %016x",Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));}
    static final class Data {JsonObject f;PrintWriter out;int calls,groundCalls,climbCalls;Vec3 velocity;
        void query(String s){out.println(s);if(calls++==f.get("fail").getAsInt())throw new Stop();}
        boolean flag(String k){return f.get(k).getAsBoolean();}
        boolean sequence(String k,int n){var a=f.getAsJsonArray(k);return a.get(Math.min(n,a.size()-1)).getAsBoolean();}}
    static final class FrictionBlock extends Block {Data d;FrictionBlock(){super(BlockBehaviour.Properties.of());}@Override public float getFriction(){d.query("friction");return single(d.f.get("friction"));}}
    static final class State extends BlockState {Block block;State(){super(null,null,null);}@Override public Block getBlock(){return block;}}
    static final class Level extends net.minecraft.server.level.ServerLevel {Data d;BlockState state;Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public BlockState getBlockState(BlockPos p){d.query("state "+p.getX()+" "+p.getY()+" "+p.getZ());return state;}
        @Override public boolean isClientSide(){d.query("client");return d.flag("client");}
        @Override public boolean hasChunkAt(BlockPos p){d.query("chunk "+p.getX()+" "+p.getY()+" "+p.getZ());return d.flag("chunk");}
        @Override public int getMinY(){d.query("min-y");return d.f.get("min_y").getAsInt();}}
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {Actor(){super(null,null);}
        Data d;
        void configure(Level l){setLevel(l);}
        @Override public BlockPos getBlockPosBelowThatAffectsMyMovement(){d.query("support");return new BlockPos(17,-64,-33);}
        @Override public boolean onGround(){d.query("ground");return d.sequence("ground",d.groundCalls++);}
        @Override public double getAttributeValue(Holder<Attribute> attribute){String n=attribute==Attributes.FRICTION_MODIFIER?"friction":attribute==Attributes.AIR_DRAG_MODIFIER?"drag":null;if(n==null)throw new AssertionError(attribute);d.query(n+"-modifier");return number(d.f.get(n+"_modifier"));}
        @Override public float getSpeed(){d.query("speed");return single(d.f.get("speed"));}
        @Override protected float getFlyingSpeed(){d.query("flying-speed");return single(d.f.get("flying_speed"));}
        @Override public float getYRot(){d.query("yaw");if(d.flag("yaw_mutate"))d.velocity=new Vec3(17,18,19);return single(d.f.get("yaw"));}
        @Override public Vec3 getDeltaMovement(){return d.velocity;}
        @Override public void setDeltaMovement(Vec3 v){d.query("set "+vector(v));d.velocity=v;}
        @Override public boolean onClimbable(){d.query("climb");return d.sequence("climb",d.climbCalls++);}
        @Override public void resetFallDistance(){d.query("reset");fallDistance=0;}
        @Override public BlockState getInBlockState(){d.query("in-block");return (d.flag("scaffolding")?Blocks.SCAFFOLDING:Blocks.STONE).defaultBlockState();}
        @Override public boolean isSuppressingSlidingDownLadder(){d.query("suppress");return d.flag("suppress");}
        @Override public void move(MoverType type,Vec3 v){if(type!=MoverType.SELF)throw new AssertionError(type);d.query("move "+vector(v));horizontalCollision=d.flag("after_horizontal");jumping=d.flag("after_jumping");wasInPowderSnow=d.flag("after_snow");if(d.flag("move_mutate"))d.velocity=vec(d.f.get("after_velocity"));}
        @Override public boolean is(TagKey<EntityType<?>> tag){if(tag!=EntityTypeTags.POWDER_SNOW_WALKABLE_MOBS)throw new AssertionError(tag);d.query("powder-tag");return d.flag("powder_tag");}
        @Override public ItemStack getItemBySlot(EquipmentSlot slot){if(slot!=EquipmentSlot.FEET)throw new AssertionError(slot);d.query("feet");return d.flag("boots")?BOOTS:ItemStack.EMPTY;}
        @Override public MobEffectInstance getEffect(Holder<MobEffect> effect){if(effect!=MobEffects.LEVITATION)throw new AssertionError(effect);d.query("levitation");return effect(d,effect);}
        @Override protected double getEffectiveGravity(){d.query("gravity");return number(d.f.get("gravity"));}
        @Override public boolean shouldDiscardFriction(){d.query("discard");return d.flag("discard");}
        @Override protected boolean omnidirectionalAirMover(){d.query("omni");return d.flag("omni");}
    }
    static final class Human extends net.minecraft.world.entity.player.Player {Human(){super(null,null);}
        Data d;
        void configure(Level l){setLevel(l);}
        @Override public BlockPos getBlockPosBelowThatAffectsMyMovement(){d.query("support");return new BlockPos(17,-64,-33);}
        @Override public boolean onGround(){d.query("ground");return d.sequence("ground",d.groundCalls++);}
        @Override public double getAttributeValue(Holder<Attribute> attribute){String n=attribute==Attributes.FRICTION_MODIFIER?"friction":attribute==Attributes.AIR_DRAG_MODIFIER?"drag":null;if(n==null)throw new AssertionError(attribute);d.query(n+"-modifier");return number(d.f.get(n+"_modifier"));}
        @Override public float getSpeed(){d.query("speed");return single(d.f.get("speed"));}
        @Override protected float getFlyingSpeed(){d.query("flying-speed");return single(d.f.get("flying_speed"));}
        @Override public float getYRot(){d.query("yaw");if(d.flag("yaw_mutate"))d.velocity=new Vec3(17,18,19);return single(d.f.get("yaw"));}
        @Override public Vec3 getDeltaMovement(){return d.velocity;}
        @Override public void setDeltaMovement(Vec3 v){d.query("set "+vector(v));d.velocity=v;}
        @Override public boolean onClimbable(){d.query("climb");return d.sequence("climb",d.climbCalls++);}
        @Override public void resetFallDistance(){d.query("reset");fallDistance=0;}
        @Override public BlockState getInBlockState(){d.query("in-block");return (d.flag("scaffolding")?Blocks.SCAFFOLDING:Blocks.STONE).defaultBlockState();}
        @Override public boolean isSuppressingSlidingDownLadder(){d.query("suppress");return d.flag("suppress");}
        @Override public void move(MoverType type,Vec3 v){if(type!=MoverType.SELF)throw new AssertionError(type);d.query("move "+vector(v));horizontalCollision=d.flag("after_horizontal");jumping=d.flag("after_jumping");wasInPowderSnow=d.flag("after_snow");if(d.flag("move_mutate"))d.velocity=vec(d.f.get("after_velocity"));}
        @Override public boolean is(TagKey<EntityType<?>> tag){if(tag!=EntityTypeTags.POWDER_SNOW_WALKABLE_MOBS)throw new AssertionError(tag);d.query("powder-tag");return d.flag("powder_tag");}
        @Override public ItemStack getItemBySlot(EquipmentSlot slot){if(slot!=EquipmentSlot.FEET)throw new AssertionError(slot);d.query("feet");return d.flag("boots")?BOOTS:ItemStack.EMPTY;}
        @Override public MobEffectInstance getEffect(Holder<MobEffect> effect){if(effect!=MobEffects.LEVITATION)throw new AssertionError(effect);d.query("levitation");return effect(d,effect);}
        @Override protected double getEffectiveGravity(){d.query("gravity");return number(d.f.get("gravity"));}
        @Override public boolean shouldDiscardFriction(){d.query("discard");return d.flag("discard");}
        @Override protected boolean omnidirectionalAirMover(){d.query("omni");return d.flag("omni");}
        @Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}
    }
    static Method method(Class<?> owner,String name,Class<?>... args)throws Exception{var m=owner.getDeclaredMethod(name,args);m.setAccessible(true);return m;}
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);var unsafe=uf.get(null);var allocate=u.getMethod("allocateInstance",Class.class);
        amplifier=MobEffectInstance.class.getDeclaredField("amplifier");amplifier.setAccessible(true);BOOTS=(ItemStack)allocate.invoke(unsafe,ItemStack.class);var stackItem=ItemStack.class.getDeclaredField("item");stackItem.setAccessible(true);stackItem.set(BOOTS,Items.LEATHER_BOOTS.builtInRegistryHolder());var stackCount=ItemStack.class.getDeclaredField("count");stackCount.setAccessible(true);stackCount.setInt(BOOTS,1);
        var air=method(LivingEntity.class,"travelInAir",Vec3.class);var relative=method(LivingEntity.class,"handleRelativeFrictionAndCalculateMovement",Vec3.class,float.class);var climb=method(LivingEntity.class,"handleOnClimbable",Vec3.class);var speed=method(LivingEntity.class,"getFrictionInfluencedSpeed",float.class);var input=method(Entity.class,"getInputVector",Vec3.class,float.class,float.class);var friction=method(LivingEntity.class,"computeModifiedFriction",float.class,float.class);var position=Entity.class.getDeclaredField("position");position.setAccessible(true);var jumping=LivingEntity.class.getDeclaredField("jumping");jumping.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){
            var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.velocity=vec(f.get("velocity"));var level=(Level)allocate.invoke(unsafe,Level.class);level.d=d;var block=(FrictionBlock)allocate.invoke(unsafe,FrictionBlock.class);block.d=d;var state=(State)allocate.invoke(unsafe,State.class);state.block=block;level.state=state;
            LivingEntity entity;if(d.flag("player")){var e=(Human)allocate.invoke(unsafe,Human.class);e.d=d;e.configure(level);entity=e;}else{var e=(Actor)allocate.invoke(unsafe,Actor.class);e.d=d;e.configure(level);entity=e;}position.set(entity,new Vec3(0,number(f.get("y")),0));entity.fallDistance=number(f.get("fall"));entity.horizontalCollision=d.flag("horizontal");jumping.setBoolean(entity,d.flag("jumping"));entity.wasInPowderSnow=d.flag("snow");
            try{Object result=switch(f.get("mode").getAsString()){case "air"->air.invoke(entity,vec(f.get("input")));case "relative"->relative.invoke(entity,vec(f.get("input")),single(f.get("friction")));case "climb"->climb.invoke(entity,vec(f.get("input")));case "speed"->speed.invoke(entity,single(f.get("friction")));case "input"->input.invoke(null,vec(f.get("input")),single(f.get("speed")),single(f.get("yaw")));case "friction"->friction.invoke(null,single(f.get("friction")),(float)number(f.get("friction_modifier")));default->throw new AssertionError();};if(result instanceof Vec3 v)out.println("vector "+vector(v));else if(result instanceof Float v)out.printf("float %08x%n",Float.floatToIntBits(v));out.println("result ok");}catch(InvocationTargetException e){if(e.getCause() instanceof Stop)out.println("result error");else throw e;}
            out.printf("final %s %016x%n",vector(d.velocity),Double.doubleToLongBits(entity.fallDistance));out.println("queries "+d.calls);
        }}
    }
}
