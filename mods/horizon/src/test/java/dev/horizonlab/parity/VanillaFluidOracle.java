package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import net.minecraft.world.level.material.FluidState;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.material.FlowingFluid;
/** Original transport for native contained fluid state and height queries. */
public final class VanillaFluidOracle {
    private static final class Stop extends RuntimeException {}
    public static void main(String[] args) throws Exception {
        if(args[0].equals("export")){VanillaBlockStateCodecOracle.main(args);return;}
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
            for(var first:BuiltInRegistries.FLUID)for(var second:BuiltInRegistries.FLUID)
                out.println("family "+BuiltInRegistries.FLUID.getKey(first)+" "+BuiltInRegistries.FLUID.getKey(second)+" "+first.isSame(second));
            for(var block:Block.BLOCK_STATE_REGISTRY) {
                var state=block.getFluidState();boolean falling=state.hasProperty(FlowingFluid.FALLING)&&state.getValue(FlowingFluid.FALLING);
                out.println("state "+Block.getId(block)+" "+BuiltInRegistries.FLUID.getKey(state.getType())+" "+state.getAmount()+" "+falling+" "+state.isFull()+" "+state.isSource()+" "+Integer.toUnsignedString(Float.floatToRawIntBits(state.getOwnHeight()))+" "+Block.getId(state.createLegacyBlock()));
            }
            for(var line:Files.readAllLines(dir.resolve("codec-inputs.txt"))) {
                var decoded=FluidState.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(line)).result();
                if(decoded.isEmpty()){out.println("codec error");continue;}
                var state=decoded.get();boolean falling=state.hasProperty(FlowingFluid.FALLING)&&state.getValue(FlowingFluid.FALLING);
                out.println("codec "+BuiltInRegistries.FLUID.getKey(state.getType())+" "+state.getAmount()+" "+falling+" "+state.isSource()+" "+Block.getId(state.createLegacyBlock()));
                var raw=FluidState.CODEC.encodeStart(JsonOps.INSTANCE,state).getOrThrow().getAsJsonObject();
                var canonical=new JsonObject();canonical.add("id",raw.get("id"));
                if(raw.has("properties")){var properties=new JsonObject();new java.util.TreeMap<>(raw.getAsJsonObject("properties").asMap()).forEach(properties::add);canonical.add("properties",properties);}
                out.println("encoded "+canonical);
            }
            for(var line:(Iterable<String>)inputs::iterator) {
                var f=JsonParser.parseString(line).getAsJsonObject();var state=Block.stateById(f.get("state").getAsInt()).getFluidState();
                var above=Block.stateById(f.get("above").getAsInt()).getFluidState();int[] calls={0};int fail=f.get("fail").getAsInt();
                var world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(proxy,method,values)->{
                    var p=(BlockPos)values[0];boolean fluid=method.getName().equals("getFluidState");
                    if(!fluid&&!method.getName().equals("getBlockState"))throw new AssertionError(method);
                    out.println((fluid?"fluid ":"block ")+p.getX()+" "+p.getY()+" "+p.getZ());
                    if(calls[0]++==fail)throw new Stop();
                    return fluid?above:(f.get("sturdy").getAsBoolean()?Blocks.STONE:Blocks.AIR).defaultBlockState();
                });
                var xyz=f.getAsJsonArray("position");var p=new BlockPos(xyz.get(0).getAsInt(),xyz.get(1).getAsInt(),xyz.get(2).getAsInt());
                for(boolean camera:new boolean[]{false,true}) {
                    try {float height=camera?state.getHeightForCamera(world,p):state.getHeight(world,p);out.println("height "+Integer.toUnsignedString(Float.floatToRawIntBits(height)));}
                    catch(Stop error){out.println("height error");}
                }
                out.println("calls "+calls[0]);
            }
        }
    }
}
