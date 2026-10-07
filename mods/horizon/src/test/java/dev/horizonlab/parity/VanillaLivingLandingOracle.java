package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.gameevent.*;
import net.minecraft.world.phys.Vec3;
/** Actual LivingEntity landing wrapper with native base fall handling. */
public final class VanillaLivingLandingOracle {
    static final class Stop extends RuntimeException {}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static Optional<BlockPos> support(JsonElement value){if(value.isJsonNull())return Optional.empty();var a=value.getAsJsonArray();return Optional.of(new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt()));}
    static Field positionField,supportField,climbableField;
    static final class Data {
        JsonObject input;PrintWriter out;int calls;Actor actor;boolean water;BlockState inputState;
        void setPosition(Vec3 p){try{positionField.set(actor,p);}catch(ReflectiveOperationException e){throw new AssertionError(e);}}
        void query(String text){out.println(text);if(calls++==input.get("fail").getAsInt())throw new Stop();}
        void mutate() {if(input.get("mutate").getAsBoolean()){try{actor.fallDistance=17.25;positionField.set(actor,new Vec3(17,18,19));supportField.set(actor,support(input.get("after_support")));}catch(ReflectiveOperationException e){throw new AssertionError(e);}}}
    }
    static final class FallBlock extends Block {
        Data d;FallBlock(){super(BlockBehaviour.Properties.of());}
        @Override public void fallOn(net.minecraft.world.level.Level level,BlockState state,BlockPos p,Entity entity,double distance){d.query(String.format("fall-on %d %d %d %016x",p.getX(),p.getY(),p.getZ(),Double.doubleToLongBits(distance)));d.mutate();}
    }
    static final class State extends BlockState {Block block;Data d;@Override public boolean isAir(){d.query("air");return d.input.get("air").getAsBoolean();}State(){super(null,null,null);}@Override public Block getBlock(){return block;}}
    static final class Level extends net.minecraft.server.level.ServerLevel {
        Data d;BlockState queried;
        private Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public <T extends net.minecraft.core.particles.ParticleOptions> int sendParticles(T particle,double x,double y,double z,int count,double dx,double dy,double dz,double speed){if(!(particle instanceof net.minecraft.core.particles.BlockParticleOption b)||b.getState()!=d.inputState||b.getType()!=net.minecraft.core.particles.ParticleTypes.BLOCK||dx!=0||dy!=0||dz!=0)throw new AssertionError(particle);d.query(String.format("particles %016x %016x %016x %d %016x",Double.doubleToLongBits(x),Double.doubleToLongBits(y),Double.doubleToLongBits(z),count,Double.doubleToLongBits(speed)));if(d.input.get("particle_mutate").getAsBoolean()){d.actor.fallDistance=number(d.input.get("after_fall"));d.water=!d.water;}return -1;}
        @Override public BlockState getBlockState(BlockPos p){d.query("state "+p.getX()+" "+p.getY()+" "+p.getZ());return queried;}
        @Override public void gameEvent(Holder<GameEvent> event,Vec3 p,GameEvent.Context context){if(event!=GameEvent.HIT_GROUND)throw new AssertionError(event);d.query(String.format("hit-ground %016x %016x %016x %s",Double.doubleToLongBits(p.x),Double.doubleToLongBits(p.y),Double.doubleToLongBits(p.z),context.affectedState()==queried?"queried":"input"));}
    }
    static final class ClientLevel extends net.minecraft.world.level.Level {
        Data d;BlockState queried;ClientLevel(){super(null,null,null,null,false,false,0L,0);}
        @Override public BlockState getBlockState(BlockPos p){d.query("state "+p.getX()+" "+p.getY()+" "+p.getZ());return queried;}
        @Override public void gameEvent(Holder<GameEvent> event,Vec3 p,GameEvent.Context context){if(event!=GameEvent.HIT_GROUND)throw new AssertionError(event);d.query(String.format("hit-ground %016x %016x %016x %s",Double.doubleToLongBits(p.x),Double.doubleToLongBits(p.y),Double.doubleToLongBits(p.z),context.affectedState()==queried?"queried":"input"));}
@Override public net.minecraft.world.clock.ClockManager clockManager(){throw new UnsupportedOperationException();}
@Override public void destroyBlockProgress(int p0,net.minecraft.core.BlockPos p1,int p2){throw new UnsupportedOperationException();}
@Override public java.util.Collection dragonParts(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes(){throw new UnsupportedOperationException();}
@Override public void explode(net.minecraft.world.entity.Entity p0,net.minecraft.world.damagesource.DamageSource p1,net.minecraft.world.level.ExplosionDamageCalculator p2,double p3,double p4,double p5,float p6,boolean p7,net.minecraft.world.level.Level.ExplosionInteraction p8,net.minecraft.core.particles.ParticleOptions p9,net.minecraft.core.particles.ParticleOptions p10,net.minecraft.util.random.WeightedList p11,net.minecraft.core.Holder p12){throw new UnsupportedOperationException();}
@Override public java.lang.String gatherChunkSourceStats(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.ticks.LevelTickAccess getBlockTicks(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.chunk.ChunkSource getChunkSource(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.entity.LevelEntityGetter getEntities(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.entity.Entity getEntity(int p0){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.ticks.LevelTickAccess getFluidTicks(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.saveddata.maps.MapItemSavedData getMapData(net.minecraft.world.level.saveddata.maps.MapId p0){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.storage.LevelData.RespawnData getRespawnData(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.scores.Scoreboard getScoreboard(){throw new UnsupportedOperationException();}
@Override public int getSeaLevel(){throw new UnsupportedOperationException();}
@Override public net.minecraft.core.Holder getUncachedNoiseBiome(int p0,int p1,int p2){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.level.border.WorldBorder getWorldBorder(){throw new UnsupportedOperationException();}
@Override public void levelEvent(net.minecraft.world.entity.Entity p0,int p1,net.minecraft.core.BlockPos p2,int p3){throw new UnsupportedOperationException();}
@Override public void playSeededSound(net.minecraft.world.entity.Entity p0,net.minecraft.world.entity.Entity p1,net.minecraft.core.Holder p2,net.minecraft.sounds.SoundSource p3,float p4,float p5,long p6){throw new UnsupportedOperationException();}
@Override public void playSeededSound(net.minecraft.world.entity.Entity p0,double p1,double p2,double p3,net.minecraft.core.Holder p4,net.minecraft.sounds.SoundSource p5,float p6,float p7,long p8){throw new UnsupportedOperationException();}
@Override public java.util.List players(){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.item.crafting.RecipeAccess recipeAccess(){throw new UnsupportedOperationException();}
@Override public void sendBlockUpdated(net.minecraft.core.BlockPos p0,net.minecraft.world.level.block.state.BlockState p1,net.minecraft.world.level.block.state.BlockState p2,int p3){throw new UnsupportedOperationException();}
@Override public void setRespawnData(net.minecraft.world.level.storage.LevelData.RespawnData p0){throw new UnsupportedOperationException();}
@Override public net.minecraft.world.TickRateManager tickRateManager(){throw new UnsupportedOperationException();}
    }
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {
        Data d;Actor(){super(null,null);}void configure(net.minecraft.world.level.Level level){setLevel(level);}
        @Override public boolean isInWater(){d.query("water");return d.water;}
        @Override public void resetFallDistance(){d.query("reset");super.resetFallDistance();}
        @Override public boolean updateFluidInteraction(){d.query("fluid-update");d.water=d.input.get("after_water").getAsBoolean();if(d.input.get("fluid_mutate").getAsBoolean())fallDistance=number(d.input.get("after_fall"));return d.input.get("fluid_result").getAsBoolean();}
        @Override protected void onChangedBlock(net.minecraft.server.level.ServerLevel level,BlockPos p){d.query("changed "+p.getX()+" "+p.getY()+" "+p.getZ());if(d.input.get("changed_mutate").getAsBoolean()){fallDistance=number(d.input.get("after_fall"));d.setPosition(new Vec3(17,18,19));}}
        @Override public double getAttributeValue(Holder<net.minecraft.world.entity.ai.attributes.Attribute> attribute){if(attribute!=net.minecraft.world.entity.ai.attributes.Attributes.SAFE_FALL_DISTANCE)throw new AssertionError(attribute);d.query("safe");return number(d.input.get("safe"));}
        @Override public BlockPos blockPosition(){d.query("block-position");if(d.input.get("block_mutate").getAsBoolean())d.setPosition(new Vec3(31,32,33));return support(d.input.get("block_position")).orElseThrow();}
        void check(double dy,boolean ground,BlockState state,BlockPos p){checkFallDamage(dy,ground,state,p);}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);positionField=Entity.class.getDeclaredField("position");positionField.setAccessible(true);supportField=Entity.class.getDeclaredField("mainSupportingBlockPos");supportField.setAccessible(true);climbableField=LivingEntity.class.getDeclaredField("lastClimbablePos");climbableField.setAccessible(true);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var d=new Data();d.input=f;d.out=out;d.water=f.get("water").getAsBoolean();net.minecraft.world.level.Level level;if(f.get("server").getAsBoolean()){var l=(Level)allocate.invoke(unsafe,Level.class);l.d=d;l.queried=Blocks.STONE.defaultBlockState();level=l;}else{var l=(ClientLevel)allocate.invoke(unsafe,ClientLevel.class);l.d=d;l.queried=Blocks.STONE.defaultBlockState();level=l;}var entity=(Actor)allocate.invoke(unsafe,Actor.class);entity.d=d;d.actor=entity;entity.configure(level);entity.fallDistance=number(f.get("fall"));var xyz=f.getAsJsonArray("position");positionField.set(entity,new Vec3(number(xyz.get(0)),number(xyz.get(1)),number(xyz.get(2))));climbableField.set(entity,Optional.of(new BlockPos(7,8,9)));supportField.set(entity,support(f.get("support")));var block=(FallBlock)allocate.invoke(unsafe,FallBlock.class);block.d=d;var state=(State)allocate.invoke(unsafe,State.class);state.block=block;state.d=d;d.inputState=state;
                try{entity.check(number(f.get("dy")),f.get("ground").getAsBoolean(),state,support(f.get("landing")).orElseThrow());out.println("result ok");}catch(Stop error){out.println("result error");}
                var p=(Vec3)positionField.get(entity);out.printf("final %016x %016x %016x %016x%n",Double.doubleToLongBits(entity.fallDistance),Double.doubleToLongBits(p.x),Double.doubleToLongBits(p.y),Double.doubleToLongBits(p.z));var selected=(Optional<?>)supportField.get(entity);if(selected.isEmpty())out.println("support none");else{var pos=(BlockPos)selected.get();out.println("support "+pos.getX()+" "+pos.getY()+" "+pos.getZ());}out.println("climbable "+((Optional<?>)climbableField.get(entity)).isPresent()+" water "+d.water);out.println("queries "+d.calls);
            }
        }
    }
}
