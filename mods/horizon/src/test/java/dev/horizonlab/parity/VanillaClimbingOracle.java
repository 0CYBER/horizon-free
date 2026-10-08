package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;import java.lang.reflect.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;import net.minecraft.nbt.*;import net.minecraft.tags.*;import net.minecraft.world.entity.*;import net.minecraft.world.level.block.*;import net.minecraft.world.level.block.state.*;
/** Actual registered LivingEntity climbing and trapdoor ladder predicates. */
public final class VanillaClimbingOracle {
    static final class Stop extends RuntimeException {}
    static Field last;
    static BlockPos pos(JsonElement v){var a=v.getAsJsonArray();return new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt());}
    static String text(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
    static final class Data {JsonObject op;PrintWriter out;int calls;BlockPos position;Actor actor;
        void query(String s){out.println(s);int n=calls++;if(n==op.get("fail").getAsInt())throw new Stop();if(n==op.get("mutate_at").getAsInt()){position=new BlockPos(31,32,33);try{last.set(actor,Optional.of(new BlockPos(-8,-9,-10)));}catch(ReflectiveOperationException e){throw new AssertionError(e);}}}}
    static final class Level extends net.minecraft.server.level.ServerLevel {Data d;Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}@Override public BlockState getBlockState(BlockPos p){d.query("below "+text(p));return Block.stateById(d.op.get("below").getAsInt());}}
    static final class Actor extends net.minecraft.world.entity.decoration.ArmorStand {Data d;Actor(){super(null,null);}void configure(Level l){setLevel(l);}
        @Override public boolean isSpectator(){d.query("spectator");return d.op.get("spectator").getAsBoolean();}
        @Override public BlockPos blockPosition(){d.query("position");return d.position;}
        @Override public BlockState getInBlockState(){d.query("in-state");return Block.stateById(d.op.get("state").getAsInt());}
        @Override public boolean isFallFlying(){d.query("gliding");return d.op.get("gliding").getAsBoolean();}}
    static void export(Path dir,JsonArray climb,JsonArray glide)throws Exception{
        var states=new ListTag();var defaults=new CompoundTag();var trapdoors=new JsonArray();var trapdoorStates=new JsonArray();var ladderStates=new JsonArray();var names=new JsonObject();
        for(var block:BuiltInRegistries.BLOCK){String name=BuiltInRegistries.BLOCK.getKey(block).toString();defaults.putInt(name,Block.getId(block.defaultBlockState()));names.addProperty(name,Block.getId(block.defaultBlockState()));if(block instanceof TrapDoorBlock)trapdoors.add(name);}
        for(var state:Block.BLOCK_STATE_REGISTRY){states.add(BlockState.FULL_CODEC.encodeStart(NbtOps.INSTANCE,state).getOrThrow());if(state.getBlock() instanceof TrapDoorBlock)trapdoorStates.add(Block.getId(state));if(state.is(Blocks.LADDER))ladderStates.add(Block.getId(state));}
        var data=new CompoundTag();data.putInt("DataVersion",net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());data.put("states",states);data.put("defaults",defaults);try(var out=new DataOutputStream(Files.newOutputStream(dir.resolve("block-registry.nbt")))){NbtIo.write(data,out);}
        var info=new JsonObject();info.addProperty("states",states.size());info.add("defaults",names);info.add("trapdoors",trapdoors);info.add("trapdoor_states",trapdoorStates);info.add("ladder_states",ladderStates);info.add("climbable",climb);info.add("glide_through",glide);Files.writeString(dir.resolve("info.json"),info.toString());
    }
    public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());var all=new ArrayList<Holder<Block>>();var climb=new ArrayList<Holder<Block>>();var glide=new ArrayList<Holder<Block>>();var cn=new JsonArray();var gn=new JsonArray();for(var block:BuiltInRegistries.BLOCK){var h=block.builtInRegistryHolder();String name=BuiltInRegistries.BLOCK.getKey(block).toString();all.add(h);if(block.defaultBlockState().is(BlockTags.CLIMBABLE)){climb.add(h);cn.add(name);}if(block.defaultBlockState().is(BlockTags.CAN_GLIDE_THROUGH)){glide.add(h);gn.add(name);}}
        if(args[0].equals("export")){export(dir,cn,gn);return;}
        var u=Class.forName("sun.misc.Unsafe");var uf=u.getDeclaredField("theUnsafe");uf.setAccessible(true);var unsafe=uf.get(null);var allocate=u.getMethod("allocateInstance",Class.class);last=LivingEntity.class.getDeclaredField("lastClimbablePos");last.setAccessible(true);int oldC=-1,oldG=-1;
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();int c=f.get("climb_tags").getAsInt(),g=f.get("glide_tags").getAsInt();if(c!=oldC||g!=oldG){BuiltInRegistries.BLOCK.prepareTagReload(new TagLoader.LoadResult<>(Registries.BLOCK,Map.of(BlockTags.CLIMBABLE,c==0?climb:c==1?List.<Holder<Block>>of():all,BlockTags.CAN_GLIDE_THROUGH,g==0?glide:g==1?List.<Holder<Block>>of():all))).apply();oldC=c;oldG=g;}
            var d=new Data();d.out=out;var level=(Level)allocate.invoke(unsafe,Level.class);level.d=d;var actor=(Actor)allocate.invoke(unsafe,Actor.class);actor.d=d;actor.configure(level);d.actor=actor;last.set(actor,Optional.of(new BlockPos(7,8,9)));
            for(var v:f.getAsJsonArray("ops")){d.op=v.getAsJsonObject();d.calls=0;d.position=pos(d.op.get("position"));try{out.println("result "+actor.onClimbable());}catch(Stop e){out.println("error");}out.println("position "+text(d.position));var p=(Optional<?>)last.get(actor);out.println(p.isEmpty()?"last none":"last "+text((BlockPos)p.get()));out.println("queries "+d.calls);}
        }}
    }
}
