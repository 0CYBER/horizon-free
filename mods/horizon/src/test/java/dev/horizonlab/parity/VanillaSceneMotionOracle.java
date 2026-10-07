package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.lang.invoke.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.effect.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.attributes.*;
import net.minecraft.world.entity.player.*;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.level.gameevent.GameEvent;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
import static dev.horizonlab.parity.VanillaRegistryCollisionScanOracle.*;

/** Real Player/LivingEntity travel, jump and Entity.move with stored attributes,
 * actual block scanning, support updates and restitution. Landing and external
 * sound/event/network consumers are controlled phase boundaries. */
public final class VanillaSceneMotionOracle {
    static Field position,history,support,noBlocks,last,stuck,delay;
    static MethodHandle aiStep;
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static void write(Field f,Object o,Object value){try{f.set(o,value);}catch(Exception e){throw new AssertionError(e);}}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static final class Border extends WorldBorder {
        @Override public boolean isInsideCloseToBorder(Entity e,AABB b){return false;}
    }
    static final class Level extends net.minecraft.server.level.ServerLevel {
        Map<Long,Chunk> chunks;WorldBorder border;
        Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public boolean isClientSide(){return false;}
        @Override public BlockGetter getChunkForCollisions(int x,int z){return chunks.get(key(x,z));}
        @Override public BlockState getBlockState(BlockPos p){var c=chunks.get(key(p.getX()>>4,p.getZ()>>4));return c==null?Blocks.AIR.defaultBlockState():c.getBlockState(p);}
        @Override public FluidState getFluidState(BlockPos p){return getBlockState(p).getFluidState();}
        @Override public List<VoxelShape> getEntityCollisions(Entity e,AABB b){return List.of();}
        @Override public List<Entity> getPushableEntities(Entity e,AABB b){return List.of();}
        @Override public WorldBorder getWorldBorder(){return border;}
        @Override public void gameEvent(Holder<GameEvent> event,Vec3 pos,GameEvent.Context context){}
    }
    static final class Actor extends Player {
        AttributeMap attrs;byte flags;float speed;
        Actor(){super(null,null);}
        void configure(Level l){setLevel(l);}
        @Override public GameType gameMode(){return GameType.SURVIVAL;}
        @Override public AttributeMap getAttributes(){return attrs;}
        @Override protected boolean getSharedFlag(int bit){return (flags&(1<<bit))!=0;}
        @Override protected void setSharedFlag(int bit,boolean v){flags=(byte)(v?flags|(1<<bit):flags&~(1<<bit));}
        @Override public void setPos(double x,double y,double z){write(position,this,new Vec3(x,y,z));setBoundingBox(EntityDimensions.scalable(.6f,1.8f).makeBoundingBox(position()));}
        @Override public BlockPos blockPosition(){return BlockPos.containing(position());}
        @Override public BlockState getInBlockState(){return level().getBlockState(blockPosition());}
        @Override public ItemStack getMainHandItem(){return ItemStack.EMPTY;}
        @Override public boolean isPassenger(){return false;}
        @Override public boolean isSwimming(){return false;}
        @Override public boolean isInWater(){return false;}
        @Override public boolean isInLava(){return false;}
        @Override public boolean isInLiquid(){return false;}
        @Override public boolean isFallFlying(){return false;}
        @Override public boolean isSpectator(){return false;}
        @Override public Pose getPose(){return Pose.STANDING;}
        @Override public boolean isDescending(){return false;}
        @Override public boolean isSuppressingBounce(){return false;}
        @Override public boolean isSuppressingSlidingDownLadder(){return false;}
        @Override public boolean isNoGravity(){return false;}
        @Override public boolean hasEffect(Holder<MobEffect> effect){return false;}
        @Override public MobEffectInstance getEffect(Holder<MobEffect> effect){return null;}
        @Override public float getSpeed(){return speed;}
        @Override public void setSpeed(float value){speed=value;}
        @Override public MoveSimulationType getMoveSimulationType(){return MoveSimulationType.SERVER_AND_CLIENT;}
        @Override public boolean isClientAuthoritative(){return false;}
        @Override public void recordMovement(MoverType mover,Vec3 movement){}
        @Override protected void checkFallDamage(double dy,boolean ground,BlockState state,BlockPos p){}
        @Override protected void playStepSound(BlockPos p,BlockState state){}
        @Override public void causeFoodExhaustion(float v){}
        @Override public void gameEvent(Holder<GameEvent> event,Entity source){}
        @Override protected boolean isImmobile(){return false;}
        @Override public LivingEntity getControllingPassenger(){return null;}
        @Override public double getFluidHeight(net.minecraft.tags.TagKey<net.minecraft.world.level.material.Fluid> tag){return 0;}
        @Override public double getFluidJumpThreshold(){return .4;}
        @Override public int getTicksFrozen(){return 0;}
        @Override public void setTicksFrozen(int value){if(value!=0)throw new AssertionError(value);}
        @Override protected void applyEffectsFromBlocks(){}
        void movementTick(Vec3 input)throws Exception {
            xxa=(float)input.x;yya=(float)input.y;zza=(float)input.z;
            try{aiStep.invoke((LivingEntity)this);}catch(Throwable t){if(t instanceof Exception e)throw e;throw new AssertionError(t);}
        }
    }
    static String pos(Object v){var o=(Optional<?>)v;return o.isEmpty()?"none":((BlockPos)o.get()).getX()+","+((BlockPos)o.get()).getY()+","+((BlockPos)o.get()).getZ();}
    static void snapshot(PrintWriter out,Actor p)throws Exception{
        AABB b=p.getBoundingBox();
        for(var v:List.of(p.position(),p.getDeltaMovement(),new Vec3(b.minX,b.minY,b.minZ),new Vec3(b.maxX,b.maxY,b.maxZ)))
            out.printf("%016x %016x %016x ",Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));
        out.printf("%s %s %s %s %s %s %s %016x %s %s %s %08x %d%n",p.horizontalCollision,p.verticalCollision,p.verticalCollisionBelow,p.minorHorizontalCollision,
            p.onGround(),noBlocks.getBoolean(p),pos(support.get(p)),Double.doubleToLongBits(p.fallDistance),pos(last.get(p)),p.needsSync,p.syncPosition,Float.floatToIntBits(p.speed),delay.getInt(p));
    }
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(Path.of(args[2]).resolve("block-tags.json"))).getAsJsonObject());
        var u=Class.forName("sun.misc.Unsafe");var f=u.getDeclaredField("theUnsafe");f.setAccessible(true);var unsafe=f.get(null);var allocate=u.getMethod("allocateInstance",Class.class);
        position=field(Entity.class,"position");history=field(Entity.class,"movementThisTick");support=field(Entity.class,"mainSupportingBlockPos");noBlocks=field(Entity.class,"onGroundNoBlocks");last=field(LivingEntity.class,"lastClimbablePos");stuck=field(Entity.class,"stuckSpeedMultiplier");
        var abilities=field(Player.class,"abilities");
        delay=field(LivingEntity.class,"noJumpDelay");
        aiStep=MethodHandles.privateLookupIn(LivingEntity.class,MethodHandles.lookup()).findSpecial(LivingEntity.class,"aiStep",MethodType.methodType(void.class),LivingEntity.class);
        var description=JsonParser.parseString(Files.readString(dir.resolve("scene.json"))).getAsJsonObject();
        var trace=new Trace();trace.out=new PrintWriter(OutputStream.nullOutputStream());trace.fail=-1;
        var level=(Level)allocate.invoke(unsafe,Level.class);level.chunks=VanillaSceneCollisionOracle.scene(description,trace);level.border=new Border();
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var row=JsonParser.parseString(line).getAsJsonObject();var p=(Actor)allocate.invoke(unsafe,Actor.class);p.configure(level);
                write(field(Entity.class,"type"),p,EntityTypes.PLAYER);
                write(field(Entity.class,"interpolationHandler"),p,InterpolationHandler.NO_OP);
                write(field(LivingEntity.class,"equipment"),p,new EntityEquipment());
                p.attrs=new AttributeMap(VanillaEntityAttributeOracle.supplier(EntityTypes.PLAYER));
                write(history,p,new ArrayDeque<>());write(support,p,Optional.empty());write(last,p,Optional.empty());write(stuck,p,Vec3.ZERO);write(abilities,p,new Abilities());
                p.setDeltaMovement(Vec3.ZERO);p.setPos(VanillaSceneCollisionOracle.vector(row.get("position")));
                for(var value:row.getAsJsonArray("ops")){
                    var op=value.getAsJsonObject();switch(op.get("mode").getAsString()){
                        case "travel"->p.travel(VanillaSceneCollisionOracle.vector(op.get("value")));
                        case "tick"->p.movementTick(VanillaSceneCollisionOracle.vector(op.get("value")));
                        case "delay"->delay.setInt(p,op.get("value").getAsInt());
                        case "move"->p.move(MoverType.SELF,VanillaSceneCollisionOracle.vector(op.get("value")));
                        case "jump"->p.jumpFromGround();
                        case "sprint"->p.setSprinting(op.get("value").getAsBoolean());
                        case "speed"->p.setSpeed(single(op.get("value")));
                        case "yaw"->p.setYRot(single(op.get("value")));
                        case "velocity"->p.setDeltaMovement(VanillaSceneCollisionOracle.vector(op.get("value")));
                        case "stuck"->write(stuck,p,VanillaSceneCollisionOracle.vector(op.get("value")));
                        case "fall"->p.fallDistance=number(op.get("value"));
                        case "jumping"->p.setJumping(op.get("value").getAsBoolean());
                        case "attribute"->{var attr=BuiltInRegistries.ATTRIBUTE.wrapAsHolder(BuiltInRegistries.ATTRIBUTE.getValue(Identifier.parse(op.get("name").getAsString())));p.getAttribute(attr).setBaseValue(number(op.get("value")));}
                        default->throw new AssertionError(op);
                    }
                    snapshot(out,p);
                }
                out.println("end");
            }
        }
    }
}
