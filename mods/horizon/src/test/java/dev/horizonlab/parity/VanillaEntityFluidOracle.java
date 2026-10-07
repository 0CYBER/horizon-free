package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;import net.minecraft.resources.Identifier;import net.minecraft.tags.*;
import net.minecraft.world.entity.*;import net.minecraft.world.level.*;import net.minecraft.world.level.biome.*;import net.minecraft.world.level.block.*;import net.minecraft.world.level.block.state.*;import net.minecraft.world.level.chunk.*;import net.minecraft.world.level.chunk.status.ChunkStatus;import net.minecraft.world.level.material.*;import net.minecraft.world.phys.*;
/** Actual entity immersion/current state machine with controlled cell providers and real chunk sections. */
public final class VanillaEntityFluidOracle {
    static final class Stop extends RuntimeException {}
    static java.util.function.Consumer<JsonObject> setup;
    static Object unsafe;static Method allocate;static Field blockPosition,level,touch,first,interactionField;
    static net.minecraft.world.level.material.Fluid[] KINDS;
    static <T>T make(Class<T> c){try{return c.cast(allocate.invoke(unsafe,c));}catch(Exception e){throw new AssertionError(e);}}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static Vec3 vec(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static String vector(Vec3 v){return String.format("%016x %016x %016x",Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));}
    static String pos(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
    static BlockPos position(JsonElement v){var a=v.getAsJsonArray();return new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt());}
    static AABB bounds(JsonElement v){if(v.isJsonNull())return null;var a=v.getAsJsonArray();return new AABB(number(a.get(0)),number(a.get(1)),number(a.get(2)),number(a.get(3)),number(a.get(4)),number(a.get(5)));}
    static final class Data {JsonObject op;PrintWriter out;int calls,pushed;Vec3 velocity;Entity entity;Chunk chunk;FluidState[] states;
        void query(String s){out.println(s);int n=calls++;if(n==op.get("fail").getAsInt())throw new Stop();if(n==op.get("mutate_at").getAsInt()){velocity=vec(op.get("after_velocity"));try{blockPosition.set(entity,position(op.get("after_block")));if(op.has("after_touch")){touch.setBoolean(entity,op.get("after_touch").getAsBoolean());first.setBoolean(entity,op.get("after_first").getAsBoolean());entity.fallDistance=number(op.get("after_fall"));}}catch(Exception e){throw new AssertionError(e);}}}
        JsonObject cell(BlockPos p){var c=op.getAsJsonObject("cells").get(pos(p));return(c==null?op.get("default"):c).getAsJsonObject();}
        boolean pushed(){query("pushed");var a=op.getAsJsonArray("pushed");return a.get(Math.min(pushed++,a.size()-1)).getAsBoolean();}
        AABB box(){query("box");return bounds(op.get("bounds"));}
        double eye(){query("eye");return number(op.get("eye"));}
        Vec3 velocity(){query("velocity");return velocity;}
        void set(Vec3 v){query("set "+vector(v));velocity=v;}
    }
    static final class Fluid extends WaterFluid.Source {Data d;int kind;
        @Override protected boolean isEmpty(){return kind==0;}
        @Override public boolean isSource(FluidState s){return kind==2||kind==4;}
        @Override public Holder.Reference<net.minecraft.world.level.material.Fluid> builtInRegistryHolder(){return KINDS[kind].builtInRegistryHolder();}
        @Override public float getHeight(FluidState s,BlockGetter w,BlockPos p){d.query("height "+pos(p));return single(d.cell(p).get("height"));}
        @Override public Vec3 getFlow(BlockGetter w,BlockPos p,FluidState s){d.query("flow "+pos(p));return vec(d.cell(p).get("flow"));}
    }
    static final class Chunk extends ProtoChunk {Data d;
        Chunk(Data d,PalettedContainerFactory factory,int min,int height){super(new ChunkPos(0,0),UpgradeData.EMPTY,LevelHeightAccessor.create(min,height),factory,null);}
        @Override public LevelChunkSection[] getSections(){if(d!=null)d.query("sections");return super.getSections();}
    }
    static final class World extends net.minecraft.server.level.ServerLevel {Data d;World(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes(){d.query("fast-lava");return net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().addConstantLayer(net.minecraft.world.attribute.EnvironmentAttributes.FAST_LAVA,previous->d.op.get("fast_lava").getAsBoolean()).build();}
        @Override public ChunkAccess getChunk(int x,int z,ChunkStatus status,boolean create){if(status!=ChunkStatus.FULL||create)throw new AssertionError();d.query("chunk "+x+" "+z);return d.op.getAsJsonArray("missing").contains(new JsonPrimitive(x+" "+z))?null:d.chunk;}
        @Override public FluidState getFluidState(BlockPos p){d.query("fluid "+pos(p));return d.op.get("mode").getAsString().equals("registered")?Block.stateById(d.cell(p).get("state").getAsInt()).getFluidState():d.states[d.cell(p).get("kind").getAsInt()];}
        @Override public net.minecraft.world.level.block.entity.BlockEntity getBlockEntity(BlockPos p){d.query("entity "+pos(p));return null;}
        @Override public BlockState getBlockState(BlockPos p){d.query("block "+pos(p));return d.op.get("mode").getAsString().equals("registered")?Block.stateById(d.cell(p).get("state").getAsInt()):(d.op.get("sturdy").getAsBoolean()?Blocks.STONE:Blocks.AIR).defaultBlockState();}
    }
    static final class Actor extends Marker {Data d;Actor(){super(null,null);}
        @Override public boolean isPushedByFluid(){return d.pushed();}
        @Override public void resetFallDistance(){d.query("reset-fall");super.resetFallDistance();}
        @Override protected void doWaterSplashEffect(){d.query("splash");}
        @Override public AABB getFluidInteractionBox(){return d.box();}@Override public double getEyeY(){return d.eye();}
        @Override public Vec3 getDeltaMovement(){return d.velocity();}@Override public void setDeltaMovement(Vec3 v){d.set(v);}
    }
    static final class Human extends net.minecraft.world.entity.player.Player {Data d;Human(){super(null,null);}
        @Override public GameType gameMode(){return GameType.SURVIVAL;}
        @Override public boolean isPushedByFluid(){return d.pushed();}
        @Override public void resetFallDistance(){d.query("reset-fall");super.resetFallDistance();}
        @Override protected void doWaterSplashEffect(){d.query("splash");}
        @Override public AABB getFluidInteractionBox(){return d.box();}@Override public double getEyeY(){return d.eye();}
        @Override public Vec3 getDeltaMovement(){return d.velocity();}@Override public void setDeltaMovement(Vec3 v){d.set(v);}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();KINDS=new net.minecraft.world.level.material.Fluid[]{Fluids.EMPTY,Fluids.FLOWING_WATER,Fluids.WATER,Fluids.FLOWING_LAVA,Fluids.LAVA};var u=Class.forName("sun.misc.Unsafe");var uf=field(u,"theUnsafe");unsafe=uf.get(null);allocate=u.getMethod("allocateInstance",Class.class);blockPosition=field(Entity.class,"blockPosition");level=field(Entity.class,"level");touch=field(Entity.class,"wasTouchingWater");first=field(Entity.class,"firstTick");interactionField=field(Entity.class,"fluidInteraction");var tick=Entity.class.getDeclaredMethod("updateFluidInteraction");tick.setAccessible(true);
        var trackers=field(EntityFluidInteraction.class,"fluidTrackers");var currents=field(EntityFluidInteraction.class,"currentAccumulators");var trackerClass=Class.forName("net.minecraft.world.entity.EntityFluidInteraction$Tracker");var kind=field(trackerClass,"fluidType");var height=field(trackerClass,"height");var eyes=field(trackerClass,"eyesInside");var currentClass=Class.forName("net.minecraft.world.entity.EntityFluidInteraction$CurrentAccumulator");var depth=field(currentClass,"height");var accumulated=field(currentClass,"accumulatedCurrent");var count=field(currentClass,"currentCount");
        var tags=new ArrayList<TagKey<net.minecraft.world.level.material.Fluid>>();for(int i=0;i<4;i++)tags.add(i==0?FluidTags.WATER:i==1?FluidTags.LAVA:TagKey.create(Registries.FLUID,Identifier.parse("horizon:immersion_"+i)));
        Holder<Biome> biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);var ids=new IdMapper<Holder<Biome>>();ids.add(biome);var factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),biome,null);
        Path dir=Path.of(args[1]);try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var ordered=new LinkedHashSet<TagKey<net.minecraft.world.level.material.Fluid>>();for(var t:f.getAsJsonArray("order"))ordered.add(tags.get(t.getAsInt()));var interaction=new EntityFluidInteraction(ordered);
            var d=new Data();d.out=out;d.states=new FluidState[5];for(int i=0;i<5;i++){var fluid=make(Fluid.class);fluid.d=d;fluid.kind=i;d.states[i]=new FluidState(fluid,new net.minecraft.world.level.block.state.properties.Property<?>[0],new Comparable<?>[0]);}var w=make(World.class);w.d=d;
            if(f.get("player").getAsBoolean()){var e=make(Human.class);e.d=d;d.entity=e;}else{var e=make(Actor.class);e.d=d;d.entity=e;}level.set(d.entity,w);interactionField.set(d.entity,interaction);if(f.has("tick_initial")){var initial=f.getAsJsonObject("tick_initial");touch.setBoolean(d.entity,initial.get("touch").getAsBoolean());first.setBoolean(d.entity,initial.get("first").getAsBoolean());d.entity.fallDistance=number(initial.get("fall"));}
            for(var value:f.getAsJsonArray("ops")){d.op=value.getAsJsonObject();var op=d.op;if(setup!=null)setup.accept(op);d.calls=0;d.pushed=0;d.velocity=vec(op.get("velocity"));blockPosition.set(d.entity,position(op.get("block")));d.entity.setBoundingBox(new AABB(0,number(op.get("base_y")),0,1,number(op.get("base_y"))+1,1));
                var bindings=new HashMap<TagKey<net.minecraft.world.level.material.Fluid>,List<Holder<net.minecraft.world.level.material.Fluid>>>();for(int t=0;t<4;t++){var members=new ArrayList<Holder<net.minecraft.world.level.material.Fluid>>();int mask=op.getAsJsonArray("tags").get(t).getAsInt();for(int k=0;k<5;k++)if((mask&(1<<k))!=0)members.add(KINDS[k].builtInRegistryHolder());bindings.put(tags.get(t),members);}BuiltInRegistries.FLUID.prepareTagReload(new TagLoader.LoadResult<>(Registries.FLUID,bindings)).apply();
                var wet=op.getAsJsonArray("wet");d.chunk=new Chunk(d,factory,op.get("min_section").getAsInt()*16,wet.size()*16);for(int i=0;i<wet.size();i++)if(wet.get(i).getAsBoolean())d.chunk.getSection(i).setBlockState(0,0,0,Blocks.WATER.defaultBlockState(),false);d.chunk.d=d;
                try{if(op.get("mode").getAsString().equals("tick"))out.println("result "+tick.invoke(d.entity));else if(op.get("mode").getAsString().equals("update")||op.get("mode").getAsString().equals("registered"))out.println("result "+interaction.update(d.entity,op.get("ignore").getAsBoolean()));else{interaction.applyCurrentTo(tags.get(op.get("tag").getAsInt()),d.entity,number(op.get("speed")));out.println("result ok");}}catch(Stop e){out.println("error");}catch(InvocationTargetException e){if(e.getCause() instanceof Stop)out.println("error");else throw e;}
                var list=(List<?>)trackers.get(interaction);out.println("trackers "+list.size());for(var t:list){Object holder=kind.get(t);int k=-1;for(int i=0;i<5;i++)if(KINDS[i].builtInRegistryHolder()==holder)k=i;out.printf("tracker %d %016x %s%n",k,Double.doubleToLongBits(height.getDouble(t)),eyes.getBoolean(t));}
                var map=(Map<?,?>)currents.get(interaction);for(var tag:ordered){var c=map.get(tag);out.printf("current %d %016x %s %d%n",tags.indexOf(tag),Double.doubleToLongBits(depth.getDouble(c)),vector((Vec3)accumulated.get(c)),count.getInt(c));}
                for(int t=0;t<4;t++)out.printf("tag %d %016x %s %s%n",t,Double.doubleToLongBits(interaction.getFluidHeight(tags.get(t))),interaction.isInFluid(tags.get(t)),interaction.isEyeInFluid(tags.get(t)));
                out.println("final "+vector(d.velocity));out.println("queries "+d.calls);if(f.has("tick_initial"))out.printf("tick %s %s %016x%n",touch.getBoolean(d.entity),first.getBoolean(d.entity),Double.doubleToLongBits(d.entity.fallDistance));
            }
        }}
    }
}
