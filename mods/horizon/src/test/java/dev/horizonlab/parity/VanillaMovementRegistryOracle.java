package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.nbt.*;
import net.minecraft.tags.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.phys.Vec3;
/** Exports raw block fields; compares registered movement queries independently. */
public final class VanillaMovementRegistryOracle {
    static final class Stop extends RuntimeException {}
    static final String[] PROPERTIES={"friction","speedFactor","jumpFactor","bounceRestitution","fallDistanceReduction"};
    static double number(JsonElement v){return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16));}
    static BlockPos position(JsonElement v){var a=v.getAsJsonArray();return new BlockPos(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt());}
    static void export(Path dir)throws Exception {
        var fields=new Field[PROPERTIES.length];for(int i=0;i<fields.length;i++){fields[i]=BlockBehaviour.class.getDeclaredField(PROPERTIES[i]);fields[i].setAccessible(true);}
        var states=new ListTag();var defaults=new CompoundTag();var definitions=new CompoundTag();
        for(var state:Block.BLOCK_STATE_REGISTRY){states.add(BlockState.FULL_CODEC.encodeStart(NbtOps.INSTANCE,state).getOrThrow());if(state==state.getBlock().defaultBlockState())defaults.putInt(BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString(),Block.getId(state));}
        for(var block:BuiltInRegistries.BLOCK){var data=new CompoundTag();for(int i=0;i<fields.length;i++)data.putFloat(PROPERTIES[i],fields[i].getFloat(block));data.putBoolean("fenceGate",block instanceof FenceGateBlock);definitions.put(BuiltInRegistries.BLOCK.getKey(block).toString(),data);}
        var info=new JsonObject();info.addProperty("states",states.size());var ids=new JsonObject();for(var block:BuiltInRegistries.BLOCK)ids.addProperty(BuiltInRegistries.BLOCK.getKey(block).toString(),Block.getId(block.defaultBlockState()));info.add("defaults",ids);Files.writeString(dir.resolve("registry-info.json"),info.toString());
        var registry=new CompoundTag();registry.putInt("DataVersion",net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());registry.put("states",states);registry.put("defaults",defaults);
        var movement=new CompoundTag();movement.putInt("DataVersion",net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());movement.put("states",states);movement.put("blocks",definitions);
        try(var out=new DataOutputStream(Files.newOutputStream(dir.resolve("block-registry.nbt")))){NbtIo.write(registry,out);}try(var out=new DataOutputStream(Files.newOutputStream(dir.resolve("movement-properties.nbt")))){NbtIo.write(movement,out);}
    }
    static final class Level extends net.minecraft.server.level.ServerLevel {
        JsonObject input;PrintWriter out;int calls;
        private Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        @Override public BlockState getBlockState(BlockPos p){out.println("state "+p.getX()+" "+p.getY()+" "+p.getZ());int call=calls++;if(call==input.get("fail").getAsInt())throw new Stop();var states=input.getAsJsonArray("states");return Block.stateById(states.get(call%states.size()).getAsInt());}
    }
    static final class Actor extends Marker {
        Actor(){super(null,null);}void configure(Level level){setLevel(level);}BlockPos cached;
        @Override public BlockPos blockPosition(){return cached;}
        BlockPos support(float offset){return getOnPos(offset);}float jump(){return getBlockJumpFactor();}float speed(){return getBlockSpeedFactor();}
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
        if(args[0].equals("export")){export(dir);return;}
        var registry=BuiltInRegistries.BLOCK;var natural=new HashMap<TagKey<Block>,List<Holder<Block>>>();var all=new ArrayList<Holder<Block>>();
        var tags=List.of(BlockTags.FENCES,BlockTags.WALLS,BlockTags.SUPPRESSES_BOUNCE);
        for(var tag:tags){var values=new ArrayList<Holder<Block>>();for(var block:registry){if(block.defaultBlockState().is(tag))values.add(block.builtInRegistryHolder());}natural.put(tag,values);}for(var block:registry)all.add(block.builtInRegistryHolder());
        var allocator=Class.forName("sun.misc.Unsafe");var field=allocator.getDeclaredField("theUnsafe");field.setAccessible(true);var unsafe=field.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var support=Entity.class.getDeclaredField("mainSupportingBlockPos");support.setAccessible(true);var pos=Entity.class.getDeclaredField("position");pos.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var state:Block.BLOCK_STATE_REGISTRY){var b=state.getBlock();out.printf("properties %d %08x %08x %08x %08x %08x %s %s %s %s %s %s%n",Block.getId(state),Float.floatToIntBits(b.getFriction()),Float.floatToIntBits(b.getSpeedFactor()),Float.floatToIntBits(b.getJumpFactor()),Float.floatToIntBits(b.getBounceRestitution()),Float.floatToIntBits(b.getFallDistanceReduction()),b instanceof FenceGateBlock,state.is(BlockTags.FENCES),state.is(BlockTags.WALLS),state.is(BlockTags.SUPPRESSES_BOUNCE),state.is(Blocks.WATER),state.is(Blocks.BUBBLE_COLUMN));}
            int active=-1;
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();int tagMode=f.get("tags").getAsInt();if(active!=tagMode){var selected=new HashMap<TagKey<Block>,List<Holder<Block>>>();for(var tag:tags)selected.put(tag,tagMode==0?natural.get(tag):tagMode==1?List.of():all);registry.prepareTagReload(new TagLoader.LoadResult<>(Registries.BLOCK,selected)).apply();active=tagMode;}
                var level=(Level)allocate.invoke(unsafe,Level.class);level.input=f;level.out=out;var entity=(Actor)allocate.invoke(unsafe,Actor.class);entity.configure(level);entity.cached=position(f.get("cached"));support.set(entity,f.get("support").isJsonNull()?Optional.empty():Optional.of(position(f.get("support"))));var v=f.getAsJsonArray("position");pos.set(entity,new Vec3(number(v.get(0)),number(v.get(1)),number(v.get(2))));
                try{int mode=f.get("mode").getAsInt();if(mode<4){BlockPos p=switch(mode){case 0->entity.support(Float.intBitsToFloat(Integer.parseUnsignedInt(f.get("offset").getAsString(),16)));case 1->entity.getOnPosLegacy();case 2->entity.getOnPos();case 3->entity.getBlockPosBelowThatAffectsMyMovement();default->throw new AssertionError();};out.printf("position %d %d %d%n",p.getX(),p.getY(),p.getZ());}else{float factor=mode==4?entity.jump():entity.speed();out.printf("factor %08x%n",Float.floatToIntBits(factor));}}
                catch(Stop error){out.println("error");}out.println("queries "+level.calls);
            }
        }
    }
}
