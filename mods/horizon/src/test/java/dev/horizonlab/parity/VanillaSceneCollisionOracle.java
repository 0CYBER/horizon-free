package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
import static dev.horizonlab.parity.VanillaRegistryCollisionScanOracle.*;

/** Actual Entity.collide and CollisionGetter.noCollision over an independently
 * constructed version of the development client's resident scene. */
public final class VanillaSceneCollisionOracle {
    static final String[][] GALLERY = {
        {"oak_stairs","facing=north","half=top","shape=inner_left"},
        {"stone_slab","type=top"}, {"anvil","facing=east"}, {"torch"}, {"lantern"},
        {"oak_fence","north=true","east=true"}, {"oak_fence_gate","facing=west","open=true"},
        {"iron_bars","north=true","south=true"}, {"cobweb"}, {"grindstone","face=wall","facing=west"},
        {"ladder","facing=west"}, {"amethyst_cluster","facing=east"}, {"stone"}, {"stone"}, {"stone"}, {"stone"}
    };
    static <T extends Comparable<T>> BlockState property(BlockState state, Property<T> key, String value) {
        return state.setValue(key,key.getValue(value).orElseThrow());
    }
    static BlockState state(String name) {
        return BuiltInRegistries.BLOCK.getValue(Identifier.parse(name)).defaultBlockState();
    }
    static void put(Map<Long,Chunk> chunks,int x,int y,int z,BlockState state) {
        chunks.get(key(x>>4,z>>4)).write(new BlockPos(x,y,z),Block.getId(state));
    }
    static Vec3 vector(JsonElement value) {
        var a=value.getAsJsonArray();return new Vec3(number(a.get(0)),number(a.get(1)),number(a.get(2)));
    }
    static HashMap<Long,Chunk> scene(JsonObject description,Trace trace)throws Exception {
        int radius=description.get("radius").getAsInt();
        if(radius<1||radius>16)throw new AssertionError("invalid scene radius");
        Holder<Biome> biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);
        var ids=new IdMapper<Holder<Biome>>();ids.add(biome);
        factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),biome,null);
        var chunks=new HashMap<Long,Chunk>();
        String[] names={"bedrock","dirt","stone","sand","oak_planks","bricks"};
        BlockState[] materials=Arrays.stream(names).map(VanillaSceneCollisionOracle::state).toArray(BlockState[]::new);
        for(int cz=-radius;cz<=radius;cz++)for(int cx=-radius;cx<=radius;cx++) {
            chunks.put(key(cx,cz),new Chunk(cx,cz,trace,null));
            for(int z=0;z<16;z++)for(int x=0;x<16;x++)for(int y=0;y<4;y++)
                put(chunks,cx*16+x,y,cz*16+z,materials[y==0?0:y<3?1:((cx+cz)&1)==0?2:3]);
        }
        for(int y=4;y<11;y++)for(int z=-10;z<=10;z++)for(int x=-10;x<=10;x++)
            if(Math.max(Math.abs(x),Math.abs(z))<=14-y)put(chunks,x,y,z,materials[(y&1)==0?4:5]);
        var gallery=description.getAsJsonArray("model_gallery");
        if(gallery.size()!=GALLERY.length || chunks.size()!=description.get("resident_chunks").getAsInt())throw new AssertionError("scene size differs");
        for(int i=0;i<GALLERY.length;i++) {
            var spec=GALLERY[i];var state=state(spec[0]);
            for(int j=1;j<spec.length;j++) {
                var p=spec[j].split("=");state=property(state,Objects.requireNonNull(state.getBlock().getStateDefinition().getProperty(p[0])),p[1]);
            }
            int x=i%4*4-7,z=i/4*4+14;
            var actual=gallery.get(i).getAsJsonObject();var p=actual.getAsJsonArray("position");
            if(actual.get("state_id").getAsInt()!=Block.getId(state) || p.get(0).getAsInt()!=x || p.get(1).getAsInt()!=4 || p.get(2).getAsInt()!=z)
                throw new AssertionError("gallery differs at "+i);
            put(chunks,x,4,z,state);
        }
        return chunks;
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Path dir=Path.of(args[1]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(Path.of(args[2]).resolve("block-tags.json"))).getAsJsonObject());
        var description=JsonParser.parseString(Files.readString(dir.resolve("scene.json"))).getAsJsonObject();
        var trace=new Trace();trace.out=new PrintWriter(OutputStream.nullOutputStream());trace.fail=-1;
        var chunks=scene(description,trace);
        var world=(CollisionGetter)Proxy.newProxyInstance(CollisionGetter.class.getClassLoader(),new Class<?>[]{CollisionGetter.class},(proxy,method,values)->{
            if(method.getName().equals("getChunkForCollisions"))return chunks.get(key((int)values[0],(int)values[1]));
            if(method.getName().equals("getBlockEntity"))return null;
            throw new AssertionError(method);
        });
        var allocator=Class.forName("sun.misc.Unsafe");var access=allocator.getDeclaredField("theUnsafe");access.setAccessible(true);
        var unsafe=access.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var level=(StepLevel)allocate.invoke(unsafe,StepLevel.class);level.trace=trace;level.world=world;level.entities=List.of();
        level.filterEntities=true;
        level.border=new StepBorder();level.border.trace=trace;
        var entity=(StepEntity)allocate.invoke(unsafe,StepEntity.class);
        var collide=net.minecraft.world.entity.Entity.class.getDeclaredMethod("collide",Vec3.class);collide.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))) {
            for(var line:(Iterable<String>)lines::iterator) {
                var f=JsonParser.parseString(line).getAsJsonObject();var bounds=new AABB(vector(f.get("min")),vector(f.get("max")));
                level.entities=new ArrayList<>();
                if(f.has("entities"))for(var shape:f.getAsJsonArray("entities"))level.entities.add(boxShape(shape));
                float step=Float.intBitsToFloat(Integer.parseUnsignedInt(f.get("step").getAsString(),16));
                entity.configure(level,bounds,step,f.get("grounded").getAsBoolean());level.context=CollisionContext.positionContext(bounds.minY);
                var movement=(Vec3)collide.invoke(entity,vector(f.get("delta")));
                boolean before=level.noCollision(entity,bounds);var moved=bounds.move(movement);
                level.context=CollisionContext.positionContext(moved.minY);boolean after=level.noCollision(entity,moved);
                out.printf("move %016x %016x %016x clear %s %s%n",Double.doubleToLongBits(movement.x),Double.doubleToLongBits(movement.y),Double.doubleToLongBits(movement.z),before,after);
            }
        }
    }
}
