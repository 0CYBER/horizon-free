package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.tags.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.level.gameevent.GameEvent;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
/** Actual Entity.move with controlled lifecycle providers and native collision resolution. */
public final class VanillaEntityMoveOracle {
    static final class Stop extends RuntimeException {}
    static Field position,stuck,history,removal;
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static Vec3 vector(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static String vec(String label,Vec3 v){return String.format("%s %016x %016x %016x",label,Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));}
    static String box(String label,AABB b){return String.format("%s %016x %016x %016x %016x %016x %016x",label,Double.doubleToLongBits(b.minX),Double.doubleToLongBits(b.minY),Double.doubleToLongBits(b.minZ),Double.doubleToLongBits(b.maxX),Double.doubleToLongBits(b.maxY),Double.doubleToLongBits(b.maxZ));}
    static String pos(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
    static void write(Field f,Object o,Object v){try{f.set(o,v);}catch(Exception e){throw new AssertionError(e);}}
    static final class Data {
        JsonObject f;PrintWriter out;int calls;Vec3 velocity;boolean grounded;Actor actor;State state;List<VoxelShape> shapes;
        boolean flag(String n){return f.get(n).getAsBoolean();}
        void query(String s){out.println(s);if(calls++==f.get("fail").getAsInt())throw new Stop();}
    }
    static final class BounceBlock extends Block {
        Data d;BounceBlock(){super(BlockBehaviour.Properties.of());}
        @Override public float getBounceRestitution(){d.query("block-bounce");return 0;}
    }
    static final class State extends BlockState {
        Data d;BounceBlock block;State(){super(null,null,null);}
        @Override public Block getBlock(){return block;}
        @Override public boolean is(TagKey<Block> tag){if(tag!=BlockTags.SUPPRESSES_BOUNCE)throw new AssertionError();d.query("block-suppresses");return false;}
    }
    static final class Border extends WorldBorder {
        Data d;@Override public boolean isInsideCloseToBorder(Entity e,AABB b){d.query(box("border-test",b));return false;}
    }
    static final class Level extends net.minecraft.server.level.ServerLevel {
        Data d;Border border;Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public boolean isClientSide(){d.query("client");return d.flag("client");}
        @Override public BlockState getBlockState(BlockPos p){d.query("block "+pos(p));return d.state;}
        @Override public WorldBorder getWorldBorder(){d.query("border");return border;}
        @Override public List<VoxelShape> getEntityCollisions(Entity e,AABB b){d.query(box("entities",b));return List.of();}
        @Override public Iterable<VoxelShape> getBlockCollisions(Entity e,AABB b){d.query(box("blocks",b));return d.shapes;}
        @Override public BlockHitResult clip(ClipContext c){d.query(vec("clip-from",c.getFrom()));d.query(vec("clip-to",c.getTo()));return d.flag("clip_hit")?new BlockHitResult(c.getTo(),Direction.UP,BlockPos.ZERO,false):BlockHitResult.miss(c.getTo(),Direction.UP,BlockPos.ZERO);}
    }
    static final class Actor extends Marker {
        Data d;Actor(){super(null,null);}void configure(Level level){setLevel(level);}
        @Override public void setPos(double x,double y,double z){var next=new Vec3(x,y,z);d.query(vec("set-position",next));setBoundingBox(getBoundingBox().move(next.subtract(position())));write(position,this,next);}
        @Override public void recordMovement(MoverType mover,Vec3 v){d.query(vec("record "+mover.ordinal(),v));}
        @Override protected Vec3 limitPistonMovement(Vec3 v){d.query(vec("piston",v));return vector(d.f.get("piston_result"));}
        @Override public Vec3 getDeltaMovement(){d.query("velocity");return d.velocity;}
        @Override public void setDeltaMovement(Vec3 v){d.query(vec("set-velocity",v));d.velocity=v;}
        @Override protected Vec3 maybeBackOffFromEdge(Vec3 v,MoverType mover){d.query(vec("edge "+mover.ordinal(),v));return d.flag("edge_override")?vector(d.f.get("edge_result")):v;}
        @Override public float maxUpStep(){return 0;}
        @Override public boolean isDescending(){return false;}
        @Override protected boolean isLocalClientAuthoritative(){d.query("local-client-authority");return d.flag("local_client");}
        @Override public boolean isClientAuthoritative(){d.query("client-authority");return d.flag("client_authority");}
        @Override public MoveSimulationType getMoveSimulationType(){d.query("simulation");return MoveSimulationType.SERVER_AND_CLIENT;}
        @Override public boolean onGround(){d.query("ground");return d.grounded;}
        @Override public void setOnGroundWithMovement(boolean below,boolean horizontal,Vec3 v){d.query(vec("ground-set "+below+" "+horizontal,v));d.grounded=below;horizontalCollision=horizontal;}
        @Override protected boolean isHorizontalCollisionMinor(Vec3 v){d.query(vec("minor",v));return d.flag("minor_result");}
        @Override public BlockPos getOnPosLegacy(){d.query("legacy");return new BlockPos(17,-64,-33);}
        @Override protected void checkFallDamage(double dy,boolean ground,BlockState s,BlockPos p){d.query(String.format("fall %016x %s %s",Double.doubleToLongBits(dy),ground,pos(p)));if(d.flag("fall_mutates")){fallDistance=3;d.velocity=new Vec3(17,18,19);horizontalCollision=d.flag("mutated_horizontal");verticalCollision=d.flag("mutated_vertical");verticalCollisionBelow=d.flag("mutated_below");}if(d.flag("fall_removes"))write(removal,this,Entity.RemovalReason.DISCARDED);}
        @Override public void resetFallDistance(){d.query("reset-fall");fallDistance=0;}
        @Override public boolean isSuppressingBounce(){d.query("suppress");return false;}
        @Override protected double getEntityBounciness(){d.query("entity-bounce");return 0;}
        @Override protected double getEffectiveGravity(){d.query("gravity");return .08;}
        @Override protected MovementEmission getMovementEmission(){d.query("emission");return d.flag("emits")?MovementEmission.ALL:MovementEmission.NONE;}
        @Override public boolean isPassenger(){d.query("passenger");return true;}
        @Override protected float getBlockSpeedFactor(){d.query("speed");return single(d.f.get("speed"));}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        position=Entity.class.getDeclaredField("position");position.setAccessible(true);stuck=Entity.class.getDeclaredField("stuckSpeedMultiplier");stuck.setAccessible(true);history=Entity.class.getDeclaredField("movementThisTick");history.setAccessible(true);removal=Entity.class.getDeclaredField("removalReason");removal.setAccessible(true);
        var record=Class.forName("net.minecraft.world.entity.Entity$Movement");var constructor=record.getDeclaredConstructor(Vec3.class,Vec3.class,Vec3.class);constructor.setAccessible(true);var from=record.getDeclaredMethod("from");from.setAccessible(true);var to=record.getDeclaredMethod("to");to.setAccessible(true);var original=record.getDeclaredMethod("axisDependentOriginalMovement");original.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){
            var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.f=f;d.out=out;d.velocity=vector(f.get("velocity"));d.grounded=d.flag("ground");var actor=(Actor)allocate.invoke(unsafe,Actor.class);actor.d=d;d.actor=actor;var level=(Level)allocate.invoke(unsafe,Level.class);level.d=d;actor.configure(level);var border=(Border)allocate.invoke(unsafe,Border.class);border.d=d;level.border=border;var state=(State)allocate.invoke(unsafe,State.class);state.d=d;var block=(BounceBlock)allocate.invoke(unsafe,BounceBlock.class);block.d=d;state.block=block;d.state=state;
            var p=vector(f.get("position"));position.set(actor,p);actor.setBoundingBox(new AABB(p.x-.3,p.y,p.z-.3,p.x+.3,p.y+1.8,p.z+.3));stuck.set(actor,vector(f.get("stuck")));actor.noPhysics=d.flag("no_physics");actor.fallDistance=number(f.get("distance"));actor.horizontalCollision=d.flag("horizontal");actor.verticalCollision=d.flag("vertical");actor.verticalCollisionBelow=d.flag("below");actor.minorHorizontalCollision=d.flag("minor");removal.set(actor,d.flag("removed")?Entity.RemovalReason.DISCARDED:null);
            var movements=new ArrayDeque<Object>();for(int i=0;i<f.get("history").getAsInt();i++)movements.add(constructor.newInstance(new Vec3(i,0,0),new Vec3(i+1,0,0),new Vec3(1,0,0)));history.set(actor,movements);d.shapes=new ArrayList<>();for(var v:f.getAsJsonArray("shapes")){var a=v.getAsJsonArray();d.shapes.add(Shapes.box(number(a.get(0)),number(a.get(1)),number(a.get(2)),number(a.get(3)),number(a.get(4)),number(a.get(5))));}
            try{actor.move(MoverType.values()[f.get("mover").getAsInt()],vector(f.get("movement")));out.println("ok");}catch(Stop e){out.println("error");}
            out.println(vec("position",actor.position()));out.println(vec("velocity-final",d.velocity));out.println(vec("stuck",(Vec3)stuck.get(actor)));out.println(box("bounds",actor.getBoundingBox()));out.printf("flags %s %s %s %s %s %s %016x%n",actor.horizontalCollision,actor.verticalCollision,actor.verticalCollisionBelow,actor.minorHorizontalCollision,d.grounded,actor.isRemoved(),Double.doubleToLongBits(actor.fallDistance));out.println("history "+movements.size());for(var r:movements){out.println(vec("from",(Vec3)from.invoke(r)));out.println(vec("to",(Vec3)to.invoke(r)));var v=(Optional<?>)original.invoke(r);out.println(v.isEmpty()?"original none":vec("original",(Vec3)v.get()));}out.println("queries "+d.calls);
        }}
    }
}
