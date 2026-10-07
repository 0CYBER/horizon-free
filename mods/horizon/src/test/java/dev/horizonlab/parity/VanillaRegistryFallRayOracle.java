package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.Identifier;
import net.minecraft.tags.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.gamerules.*;
import net.minecraft.world.level.material.*;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
/** Registered fall-reset rays with actual block interactions, fluid shapes and tags. */
public final class VanillaRegistryFallRayOracle {
    static final class Stop extends RuntimeException {}
    static Object unsafe;static Method allocate;
    static <T>T make(Class<T> type){try{return type.cast(allocate.invoke(unsafe,type));}catch(Exception e){throw new AssertionError(e);}}
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static Vec3 vector(JsonElement v){var a=v.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));}
    static String pos(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
    static final class Data {
        JsonObject f,op;PrintWriter out;int calls;BlockPos position;Rules rules;
        void query(String s){out.println(s);if(calls++==op.get("fail").getAsInt())throw new Stop();}
        BlockState at(BlockPos p){return Block.stateById(p.equals(position)?f.get("state").getAsInt():p.equals(position.above())?op.get("above").getAsInt():0);}
        BlockState block(BlockPos p){query("block "+pos(p));return at(p);}
        FluidState fluid(BlockPos p){query("fluid "+pos(p));return at(p).getFluidState();}
    }
    static final class Rules extends GameRules {
        Data d;Rules(){super(net.minecraft.world.flag.FeatureFlagSet.of());}
        @SuppressWarnings("unchecked") @Override public <T>T get(GameRule<T> rule){if(rule!=GameRules.PLAYERS_NETHER_PORTAL_DEFAULT_DELAY)throw new AssertionError();d.query("portal-delay");return (T)Integer.valueOf(d.f.get("delay").getAsInt());}
    }
    static final class ServerWorld extends net.minecraft.server.level.ServerLevel {
        Data d;ServerWorld(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public BlockState getBlockState(BlockPos p){return d.block(p);}
        @Override public FluidState getFluidState(BlockPos p){return d.fluid(p);}
        @Override public GameRules getGameRules(){d.query("rules");return d.rules;}
    }
    static final class Actor extends Marker {
        Data d;Actor(){super(null,null);}
        @Override public boolean is(EntityType<?> type){if(type!=EntityTypes.PLAYER)throw new AssertionError();d.query("player");return d.f.get("player").getAsBoolean();}
        @Override public boolean isDescending(){return false;}
    }
    private static void bindFluidTags(JsonObject resources) {
        var registry=BuiltInRegistries.FLUID;
        var loader=new TagLoader<Holder<Fluid>>((id,required)->registry.get(id).map(value->(Holder<Fluid>)value),"tags/fluid");
        var definitions=new HashMap<Identifier,List<TagLoader.EntryWithSource>>();
        for(var item:resources.entrySet()) {
            var file=TagFile.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(item.getValue().getAsString())).getOrThrow();
            var entries=new ArrayList<TagLoader.EntryWithSource>();
            for(var entry:file.entries())entries.add(new TagLoader.EntryWithSource(entry,"vanilla"));
            definitions.put(Identifier.parse(item.getKey()),entries);
        }
        var tags=new HashMap<TagKey<Fluid>,List<Holder<Fluid>>>();
        loader.build(definitions).forEach((id,values)->tags.put(TagKey.create(Registries.FLUID,id),List.copyOf(values)));
        if(tags.size()!=resources.size())throw new AssertionError("incomplete fluid tags");
        registry.prepareTagReload(new TagLoader.LoadResult<>(Registries.FLUID,tags)).apply();
    }

    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());bindFluidTags(JsonParser.parseString(Files.readString(dir.resolve("fluid-tags.json"))).getAsJsonObject());
        var blocks=new ArrayList<Holder<Block>>();var reset=new ArrayList<Holder<Block>>();var fluids=new ArrayList<Holder<Fluid>>();var water=new ArrayList<Holder<Fluid>>();var resetNames=new JsonArray();var waterNames=new JsonArray();for(var b:BuiltInRegistries.BLOCK){blocks.add(b.builtInRegistryHolder());if(b.defaultBlockState().is(BlockTags.FALL_DAMAGE_RESETTING)){reset.add(b.builtInRegistryHolder());resetNames.add(BuiltInRegistries.BLOCK.getKey(b).toString());}}for(var f:BuiltInRegistries.FLUID){fluids.add(f.builtInRegistryHolder());if(f.is(FluidTags.WATER)){water.add(f.builtInRegistryHolder());waterNames.add(BuiltInRegistries.FLUID.getKey(f).toString());}}
        if(args[0].equals("export")){VanillaInteractionShapeOracle.main(args);var bound=new JsonObject();bound.add("reset",resetNames);bound.add("water",waterNames);Files.writeString(dir.resolve("bound-tags.json"),bound.toString());var info=JsonParser.parseString(Files.readString(dir.resolve("info.json"))).getAsJsonObject();var defaults=new JsonObject();for(var b:BuiltInRegistries.BLOCK)defaults.addProperty(BuiltInRegistries.BLOCK.getKey(b).toString(),Block.getId(b.defaultBlockState()));info.add("defaults",defaults);Files.writeString(dir.resolve("info.json"),info.toString());return;}
        var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);unsafe=field.get(null);allocate=allocator.getMethod("allocateInstance",Class.class);var entityPosition=Entity.class.getDeclaredField("position");entityPosition.setAccessible(true);var caches=FlowingFluid.class.getDeclaredField("shapes");caches.setAccessible(true);int oldBlocks=-1,oldFluids=-1;
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){
            var f=JsonParser.parseString(line).getAsJsonObject();int bt=f.get("block_tags").getAsInt(),ft=f.get("fluid_tags").getAsInt();if(bt!=oldBlocks){BuiltInRegistries.BLOCK.prepareTagReload(new TagLoader.LoadResult<>(Registries.BLOCK,Map.of(BlockTags.FALL_DAMAGE_RESETTING,bt==0?reset:bt==1?List.<Holder<Block>>of():blocks))).apply();oldBlocks=bt;}if(ft!=oldFluids){BuiltInRegistries.FLUID.prepareTagReload(new TagLoader.LoadResult<>(Registries.FLUID,Map.of(FluidTags.WATER,ft==0?water:ft==1?List.<Holder<Fluid>>of():fluids))).apply();oldFluids=ft;}
            for(var fluid:List.of(Fluids.WATER,Fluids.FLOWING_WATER,Fluids.LAVA,Fluids.FLOWING_LAVA))((Map<?,?>)caches.get(fluid)).clear();var d=new Data();d.f=f;d.out=out;var p=f.getAsJsonArray("position");d.position=new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());d.rules=make(Rules.class);d.rules.d=d;BlockGetter world;if(f.get("server").getAsBoolean()){var w=make(ServerWorld.class);w.d=d;world=w;}else{world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(proxy,method,values)->{if(method.getName().equals("getBlockState"))return d.block((BlockPos)values[0]);if(method.getName().equals("getFluidState"))return d.fluid((BlockPos)values[0]);if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,values);throw new AssertionError(method);});}
            CollisionContext context=CollisionContext.empty();if(f.get("entity").getAsBoolean()){var actor=make(Actor.class);actor.d=d;entityPosition.set(actor,Vec3.ZERO);context=CollisionContext.of(actor);}
            for(var value:f.getAsJsonArray("ops")){d.op=value.getAsJsonObject();d.calls=0;var c=new ClipContext(vector(d.op.get("from")),vector(d.op.get("to")),ClipContext.Block.FALLDAMAGE_RESETTING,ClipContext.Fluid.WATER,context);
                try{var hit=world.clip(c);var v=hit.getLocation();out.printf("hit %s %016x %016x %016x %s %s %s%n",hit.getType()==HitResult.Type.MISS,Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z),hit.getDirection().getSerializedName(),pos(hit.getBlockPos()),hit.isInside());}catch(Stop e){out.println("error");}int count=0;for(var fluid:List.of(Fluids.WATER,Fluids.FLOWING_WATER,Fluids.LAVA,Fluids.FLOWING_LAVA))count+=((Map<?,?>)caches.get(fluid)).size();out.println("cached "+count);out.println("queries "+d.calls);
            }
        }}
    }
}
