package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.core.BlockPos;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;
/** Runs native BlockCollisions with controlled block metadata and traced providers. */
public final class VanillaBlockCollisionScanOracle {
    static final class Stop extends RuntimeException {}
    static final class Trace {
        PrintWriter out;int calls,fail;CollisionGetter global;
        void query(String value){out.println(value);if(calls++==fail)throw new Stop();}
    }
    public static class State extends BlockState {
        int id;boolean large,moving,suffocating;VoxelShape shape;Trace trace;BlockGetter readFrom;
        public State(){super(null,null,null);}
        @Override public VoxelShape getCollisionShape(BlockGetter world,BlockPos p,CollisionContext context){if(world!=trace.global)throw new AssertionError("wrong collision world");trace.query("shape "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+id);return shape;}
        @Override public Block getBlock(){return moving?Blocks.MOVING_PISTON:Blocks.STONE;}
        @Override public boolean hasLargeCollisionShape(){return large;}
        @Override public boolean isSuffocating(BlockGetter world,BlockPos p){if(world!=readFrom||world==trace.global)throw new AssertionError("wrong suffocation world");trace.query("suffocating "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+id);return suffocating;}
    }
    public static class SupportingEntity extends net.minecraft.world.entity.Marker {
        public SupportingEntity(){super(null,null);}
        @Override public boolean isDescending(){return false;}
    }
    static VoxelShape shape(int index){return switch(index){
        case 0->Shapes.empty();case 1->Shapes.block();case 2->Shapes.block().move(0,0,0);
        case 3->Shapes.box(0,0,0,1,0.5,1);case 4->Shapes.box(-0.25,-0.5,-0.25,1.25,1.5,1.25);
        case 5->Shapes.box(0,0,0,1e-8,1,1);
        case 6->Shapes.or(Shapes.box(0,0,0,0.25,1,0.25),Shapes.box(0.75,0,0.75,1,1,1));
        default->throw new AssertionError(index);
    };}
    static double number(JsonElement value){return Double.longBitsToDouble(Long.parseUnsignedLong(value.getAsString(),16));}
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var allocator=Class.forName("sun.misc.Unsafe");var access=allocator.getDeclaredField("theUnsafe");access.setAccessible(true);var unsafe=access.get(null);var allocate=allocator.getMethod("allocateInstance",Class.class);
        var entityPosition=net.minecraft.world.entity.Entity.class.getDeclaredField("position");entityPosition.setAccessible(true);
        var emit=VanillaFaceSupportOracle.class.getDeclaredMethod("emit",PrintWriter.class,VoxelShape.class);emit.setAccessible(true);
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var f=JsonParser.parseString(line).getAsJsonObject();var trace=new Trace();trace.out=out;trace.fail=f.get("fail").getAsInt();
                var palette=f.getAsJsonArray("palette");var states=new State[palette.size()];
                for(int i=0;i<states.length;i++){var v=palette.get(i).getAsJsonArray();var state=(State)allocate.invoke(unsafe,State.class);state.id=i;state.shape=shape(v.get(0).getAsInt());state.large=v.get(1).getAsBoolean();state.moving=v.get(2).getAsBoolean();state.suffocating=v.get(3).getAsBoolean();state.trace=trace;states[i]=state;}
                int seed=f.get("seed").getAsInt(),missing=f.get("missing").getAsInt();
                var world=(CollisionGetter)Proxy.newProxyInstance(CollisionGetter.class.getClassLoader(),new Class<?>[]{CollisionGetter.class},(proxy,method,values)->{
                    if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,values);
                    if(!method.getName().equals("getChunkForCollisions"))throw new AssertionError(method);
                    int cx=(int)values[0],cz=(int)values[1];trace.query("chunk "+cx+" "+cz);
                    if(missing==1||missing==2&&((cx^cz)&1)!=0)return null;
                    return Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},(chunk,m,a)->{
                        if(!m.getName().equals("getBlockState"))throw new AssertionError(m);
                        var p=(BlockPos)a[0];trace.query("block "+p.getX()+" "+p.getY()+" "+p.getZ());
                        var state=states[((p.getX()*3+p.getY()*5+p.getZ()*7+seed)&Integer.MAX_VALUE)%states.length];state.readFrom=(BlockGetter)chunk;return state;
                    });
                });trace.global=world;
                var context=new CollisionContext(){
                    public boolean isDescending(){throw new AssertionError();}
                    public boolean isAbove(VoxelShape s,BlockPos p,boolean d){throw new AssertionError();}
                    public boolean isHoldingItem(Item i){throw new AssertionError();}
                    public boolean alwaysCollideWithFluid(){throw new AssertionError();}
                    public boolean canStandOnFluid(FluidState a,FluidState b){throw new AssertionError();}
                    public VoxelShape getCollisionShape(BlockState state,CollisionGetter collision,BlockPos p){if(collision!=world)throw new AssertionError("wrong collision world");var s=(State)state;trace.query("shape "+p.getX()+" "+p.getY()+" "+p.getZ()+" "+s.id);return s.shape;}
                };
                var v=f.getAsJsonArray("box");var box=new AABB(number(v.get(0)),number(v.get(1)),number(v.get(2)),number(v.get(3)),number(v.get(4)),number(v.get(5)));
                if(f.has("support_position")){
                    var entity=(SupportingEntity)allocate.invoke(unsafe,SupportingEntity.class);var pos=f.getAsJsonArray("support_position");entityPosition.set(entity,new Vec3(number(pos.get(0)),number(pos.get(1)),number(pos.get(2))));
                    try{var support=world.findSupportingBlock(entity,box);if(support.isEmpty())out.println("support none");else{var p=support.get();out.println("support "+p.getX()+" "+p.getY()+" "+p.getZ());}}catch(Stop error){out.println("support error");}
                    out.println("done support "+trace.calls);continue;
                }
                var scan=new BlockCollisions<VoxelShape>(world,context,box,f.get("suffocating").getAsBoolean(),(p,s)->{out.println("hit "+p.getX()+" "+p.getY()+" "+p.getZ());return s;});
                int hits=0,limit=f.get("limit").getAsInt();
                try{while(hits<limit&&scan.hasNext()){emit.invoke(null,out,scan.next());hits++;}}catch(Stop error){out.println("error");}
                out.println("done "+hits+" "+trace.calls);
            }
        }
    }
}
