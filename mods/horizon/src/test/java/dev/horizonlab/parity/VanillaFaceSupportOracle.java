package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
/** Native face slicing and support algorithms over caller-specified shape data. */
public final class VanillaFaceSupportOracle {
    private static final class SupportBlock extends Block {
        VoxelShape shape;int calls;
        SupportBlock(){super(BlockBehaviour.Properties.of().setId(ResourceKey.create(Registries.BLOCK,Identifier.parse("horizon:fixture_support"))));}
        @Override protected VoxelShape getBlockSupportShape(BlockState state,BlockGetter world,BlockPos pos){calls++;return shape;}
    }
    private static double number(JsonElement value){return Double.longBitsToDouble(Long.parseUnsignedLong(value.getAsString(),16));}
    private static double[] numbers(JsonArray values){double[] result=new double[values.size()];for(int i=0;i<result.length;i++)result[i]=number(values.get(i));return result;}
    private static void values(PrintWriter out,String label,double... values){out.print(label);for(double value:values)out.printf(" %016x",Double.doubleToLongBits(value));out.println();}
    private static void emit(PrintWriter out,VoxelShape shape) {
        var boxes=shape.toAabbs();out.println("shape "+shape.isEmpty()+" "+boxes.size()+" "+(shape instanceof CubeVoxelShape));
        int axes=0;for(var axis:Direction.Axis.values()){var coords=shape.getCoords(axis);if(coords instanceof CubePointRange)axes|=1<<axis.ordinal();values(out,axis.getName(),coords.toDoubleArray());}out.println("axes "+axes);
        values(out,"extrema",shape.min(Direction.Axis.X),shape.min(Direction.Axis.Y),shape.min(Direction.Axis.Z),shape.max(Direction.Axis.X),shape.max(Direction.Axis.Y),shape.max(Direction.Axis.Z));
        if(shape.isEmpty())out.println("bounds none");else {var b=shape.bounds();values(out,"bounds",b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ);}
        for(var b:boxes)values(out,"box",b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ);
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var array=ArrayVoxelShape.class.getDeclaredConstructor(DiscreteVoxelShape.class,double[].class,double[].class,double[].class);array.setAccessible(true);
        // Geometry-only snapshots bypass registration and block construction.
        var allocator=Class.forName("sun.misc.Unsafe");var access=allocator.getDeclaredField("theUnsafe");access.setAccessible(true);var unsafe=access.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var block=(SupportBlock)allocate.invoke(unsafe,SupportBlock.class);var state=(BlockState)allocate.invoke(unsafe,BlockState.class);
        var owner=StateHolder.class.getDeclaredField("owner");owner.setAccessible(true);owner.set(state,block);Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
            for(var line:(Iterable<String>)inputs::iterator) {
                var f=JsonParser.parseString(line).getAsJsonObject();VoxelShape shape;
                if(f.has("coords")) {
                    var coords=f.getAsJsonArray("coords");double[] x=numbers(coords.get(0).getAsJsonArray()),y=numbers(coords.get(1).getAsJsonArray()),z=numbers(coords.get(2).getAsJsonArray());
                    int nx=x.length-1,ny=y.length-1,nz=z.length-1;var bits=new BitSetDiscreteVoxelShape(nx,ny,nz);var occupied=f.getAsJsonArray("occupied");
                    for(int i=0;i<nx;i++)for(int j=0;j<ny;j++)for(int k=0;k<nz;k++)if(occupied.get((i*ny+j)*nz+k).getAsBoolean())bits.fill(i,j,k);
                    shape=f.get("cube").getAsBoolean()?new CubeVoxelShape(bits):array.newInstance(bits,x,y,z);
                } else {shape=Shapes.empty();for(var v:f.getAsJsonArray("boxes")){var b=numbers(v.getAsJsonArray());shape=Shapes.or(shape,Shapes.create(new AABB(b[0],b[1],b[2],b[3],b[4],b[5])));}}
                if(f.has("move")){var v=numbers(f.getAsJsonArray("move"));shape=shape.move(v[0],v[1],v[2]);}
                for(var direction:Direction.values()) {
                    var face=shape.getFaceShape(direction);out.println("face "+direction.ordinal()+" "+(face==shape)+" "+(face==shape.getFaceShape(direction)));emit(out,face);
                    block.shape=shape;block.calls=0;out.println("support "+SupportType.FULL.isSupporting(state,EmptyBlockGetter.INSTANCE,BlockPos.ZERO,direction)+" "+SupportType.CENTER.isSupporting(state,EmptyBlockGetter.INSTANCE,BlockPos.ZERO,direction)+" "+SupportType.RIGID.isSupporting(state,EmptyBlockGetter.INSTANCE,BlockPos.ZERO,direction)+" "+block.calls);
                    for(var nested:Direction.values()){var second=face.getFaceShape(nested);out.println("nested "+nested.ordinal()+" "+(second==face));emit(out,second);}
                }
            }
        }
    }
}
