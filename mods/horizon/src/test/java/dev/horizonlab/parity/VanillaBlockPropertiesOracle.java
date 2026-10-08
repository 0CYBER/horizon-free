package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.Direction;
import net.minecraft.nbt.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.*;
/** Imports definition inputs only; vanilla computes the independently compared solidity results. */
public final class VanillaBlockPropertiesOracle {
    private static Field field(Class<?> cls,String name) throws Exception {var f=cls.getDeclaredField(name);f.setAccessible(true);return f;}
    private static VoxelShape shape(JsonElement value) {
        var a=value.getAsJsonArray();double[] v=new double[6];for(int i=0;i<6;i++)v[i]=Double.longBitsToDouble(Long.parseUnsignedLong(a.get(i).getAsString(),16));
        return Shapes.create(new AABB(v[0],v[1],v[2],v[3],v[4],v[5]));
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var properties=field(BlockBehaviour.class,"properties");var on=field(BlockBehaviour.Properties.class,"forceSolidOn");var off=field(BlockBehaviour.Properties.class,"forceSolidOff");var replace=field(BlockBehaviour.Properties.class,"replaceable");var liquid=field(BlockBehaviour.Properties.class,"liquid");var occlude=field(BlockBehaviour.Properties.class,"canOcclude");
        var cache=field(BlockBehaviour.BlockStateBase.class,"cache");var collision=field(cache.getType(),"collisionShape");
        var calculate=BlockBehaviour.BlockStateBase.class.getDeclaredMethod("calculateSolid");calculate.setAccessible(true);
        Path dir=Path.of(args[1]);
        if(args[0].equals("export")) {
            VanillaCollisionOracle.main(new String[]{"generate",args[1]});
            var states=new ListTag();var blocks=new CompoundTag();
            for(var state:Block.BLOCK_STATE_REGISTRY)states.add(BlockState.FULL_CODEC.encodeStart(NbtOps.INSTANCE,state).getOrThrow());
            for(var block:BuiltInRegistries.BLOCK) {
                var p=properties.get(block);blocks.putByte(BuiltInRegistries.BLOCK.getKey(block).toString(),(byte)((on.getBoolean(p)?1:0)|(off.getBoolean(p)?2:0)|(replace.getBoolean(p)?4:0)|(liquid.getBoolean(p)?8:0)|(occlude.getBoolean(p)?16:0)));
            }
            var root=new CompoundTag();root.putInt("DataVersion",net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());root.put("states",states);root.put("blocks",blocks);root.put("occlusion",VanillaCollisionOracle.occlusionDefinitions());
            try(var out=new DataOutputStream(Files.newOutputStream(dir.resolve("block-properties.nbt")))){NbtIo.write(root,out);}return;
        }
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")))) {
            for(var state:Block.BLOCK_STATE_REGISTRY) {
                var p=properties.get(state.getBlock());
                out.printf("state %d %s %s %s %s%n",Block.getId(state),on.getBoolean(p),off.getBoolean(p),state.canBeReplaced(),state.isSolid());
                out.printf("render %s %s %s%n",state.liquid(),state.canOcclude(),state.isSolidRender());
                int mask=0;for(var direction:Direction.values())if(Block.isShapeFullBlock(state.getFaceOcclusionShape(direction)))mask|=1<<direction.ordinal();out.println("occlusion "+mask);
                var c=cache.get(state);var s=c==null?null:(VoxelShape)collision.get(c);
                if(s==null)out.println("cache none");else if(s.isEmpty())out.println("cache empty");else {
                    var b=s.bounds();out.printf("cache %016x %016x %016x %016x %016x %016x%n",Double.doubleToRawLongBits(b.minX),Double.doubleToRawLongBits(b.minY),Double.doubleToRawLongBits(b.minZ),Double.doubleToRawLongBits(b.maxX),Double.doubleToRawLongBits(b.maxY),Double.doubleToRawLongBits(b.maxZ));
                }
            }
            var state=Blocks.STONE.defaultBlockState();var p=properties.get(Blocks.STONE);var savedCache=cache.get(state);var savedShape=collision.get(savedCache);boolean savedOn=on.getBoolean(p),savedOff=off.getBoolean(p);
            try(var inputs=Files.lines(dir.resolve("inputs.txt"))) {
                for(var line:(Iterable<String>)inputs::iterator) {
                    var f=JsonParser.parseString(line).getAsJsonObject();VoxelShape s=Shapes.empty();for(var value:f.getAsJsonArray("shape"))s=Shapes.or(s,shape(value));
                    on.setBoolean(p,f.get("on").getAsBoolean());off.setBoolean(p,f.get("off").getAsBoolean());cache.set(state,f.get("cached").getAsBoolean()?savedCache:null);collision.set(savedCache,s);
                    out.println("solid "+calculate.invoke(state));
                    out.println("render "+Block.isShapeFullBlock(f.get("occludes").getAsBoolean()?s:Shapes.empty()));
                    int mask=0;if(f.get("occludes").getAsBoolean())for(var direction:Direction.values())if(Block.isShapeFullBlock(s.getFaceShape(direction)))mask|=1<<direction.ordinal();out.println("occlusion "+mask);
                }
            } finally {cache.set(state,savedCache);collision.set(savedCache,savedShape);on.setBoolean(p,savedOn);off.setBoolean(p,savedOff);}
        }
    }
}
