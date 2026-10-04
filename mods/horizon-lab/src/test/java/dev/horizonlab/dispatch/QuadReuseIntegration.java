package dev.horizonlab.dispatch;
import com.seibel.distanthorizons.core.dataObjects.render.bufferBuilding.LodQuadBuilder;
import com.seibel.distanthorizons.core.dataObjects.render.bufferBuilding.BufferQuad;
import com.seibel.distanthorizons.core.enums.EDhDirection;
import java.lang.management.ManagementFactory;
import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.Random;
import java.security.MessageDigest;
import java.util.HexFormat;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLifecycleEvents;
public final class QuadReuseIntegration implements ClientModInitializer {
 static volatile long sink;
 static Field opaque,transparent,pool;
 public void onInitializeClient(){ClientLifecycleEvents.CLIENT_STARTED.register(client->{try{exercise();}catch(Exception e){throw new AssertionError("Applied quad reuse fixture failed",e);}System.exit(0);});}
 static Field field(String name)throws Exception{var f=LodQuadBuilder.class.getDeclaredField(name);f.setAccessible(true);return f;}
 static void add(LodQuadBuilder b,EDhDirection face,int x,int y,int z,int width,int color,int tile,int material,int sky,int block){
  if(face==EDhDirection.UP)b.addQuadUp((short)x,(short)y,(short)z,(short)width,color,(short)tile,(byte)material,(byte)sky,(byte)block);
  else if(face==EDhDirection.DOWN)b.addQuadDown((short)x,(short)y,(short)z,(short)width,color,(short)tile,(byte)material,(byte)sky,(byte)block);
  else b.addQuadAdj(face,(short)x,(short)y,(short)z,(short)width,(short)1,color,(short)tile,(byte)material,(byte)sky,(byte)block);
 }
 static long fingerprint(LodQuadBuilder b,MessageDigest digest)throws Exception{
  long count=0;var identities=new java.util.IdentityHashMap<BufferQuad,Boolean>();
  for(Field field:new Field[]{opaque,transparent}){
   digest.update(field.getName().getBytes(java.nio.charset.StandardCharsets.UTF_8));
   Object[] lists=(Object[])field.get(b);
   for(int face=0;face<lists.length;face++){
    var list=(ArrayList<?>)lists[face];
    digest.update((byte)face);
    for(int shift=24;shift>=0;shift-=8)digest.update((byte)(list.size()>>>shift));
    for(Object item:list){
     BufferQuad quad=(BufferQuad)item;
     if(identities.put(quad,true)!=null)throw new AssertionError("Aliased live quad");
     long[] values={quad.x,quad.y,quad.z,quad.widthEastWest,quad.widthNorthSouthOrHeight,quad.color,quad.textureTileId,quad.irisBlockMaterialId,quad.skyLight,quad.blockLight,quad.direction.ordinal(),quad.hasError?1:0,quad.sortKeyEastWest,quad.sortKeyNorthSouth};
     for(long value:values)for(int shift=56;shift>=0;shift-=8)digest.update((byte)(value>>>shift));
     count++;
    }
   }
  }
  return count;
 }
 static void batch(LodQuadBuilder b,int rows){
  for(int y=0;y<rows;y++)for(int x=0;x<64;x++)add(b,EDhDirection.NORTH,x,y,0,1,0xff113355,3,2,15,4);
 }
 static String threadOwnership() throws Exception {
  var executor=java.util.concurrent.Executors.newFixedThreadPool(4);
  var owners=java.util.Collections.synchronizedMap(new java.util.IdentityHashMap<BufferQuad,Integer>());
  var barrier=new java.util.concurrent.CyclicBarrier(4);
  var futures=new ArrayList<java.util.concurrent.Future<String>>();
  try {
   for(int worker=0;worker<4;worker++){
    final int ownerId=worker;
    futures.add(executor.submit(()->{
     var digest=MessageDigest.getInstance("SHA-256");
     var builder=LodQuadBuilder.getBuilder(false,null);
     for(int repeat=0;repeat<16;repeat++){
      barrier.await(20,java.util.concurrent.TimeUnit.SECONDS);
      batch(builder,8);
      for(Field field:new Field[]{opaque,transparent})for(Object list:(Object[])field.get(builder))for(Object item:(ArrayList<?>)list){
       var quad=(BufferQuad)item;
       Integer previous=owners.putIfAbsent(quad,ownerId);
       if(previous!=null && previous!=ownerId)throw new AssertionError("Quad shared across builder threads");
      }
      fingerprint(builder,digest);builder.mergeQuads();fingerprint(builder,digest);builder.close();
     }
     return HexFormat.of().formatHex(digest.digest());
    }));
   }
   String expected=null;
   for(var future:futures){String actual=future.get(30,java.util.concurrent.TimeUnit.SECONDS);if(expected!=null&&!expected.equals(actual))throw new AssertionError("Concurrent builder geometry differed");expected=actual;}
   return expected;
  }finally{executor.shutdownNow();if(!executor.awaitTermination(10,java.util.concurrent.TimeUnit.SECONDS))throw new AssertionError("Fixture builder workers did not stop");}
 }
 static void exercise()throws Exception{
  boolean enabled=dev.horizonlab.mixin.HorizonMixinPlugin.quadReuseEnabled();boolean applied=java.util.Arrays.stream(LodQuadBuilder.class.getDeclaredMethods()).anyMatch(m->m.getName().contains("horizon$freshQuad"));if(applied!=enabled)throw new AssertionError("Applied quad reuse state mismatch");
  opaque=field("opaqueQuads");transparent=field("transparentQuads");pool=field("bufferQuadCacheList");var digest=MessageDigest.getInstance("SHA-256");var rng=new Random(77181);long quads=0;int faces=0;
  for(int round=0;round<36;round++){
   var b=LodQuadBuilder.getBuilder((round&1)!=0,null);
   if(round==20){var poisoned=new BufferQuad();poisoned.hasError=true;((ArrayList<BufferQuad>)pool.get(b)).add(poisoned);}
   for(EDhDirection face:EDhDirection.values())for(int y=0;y<8;y++)for(int x=0;x<32;x++){
    int color=(round%3==0?0x88000000:0xff000000)|0x112233;
    int tile=round%3==1?rng.nextInt(256):3,sky=round%3==2?rng.nextInt(16):15;
    add(b,face,x,y,round%3==0?rng.nextInt(16):0,1,color,tile,round%4,sky,round%16);faces++;
   }
   quads+=fingerprint(b,digest);b.mergeQuads();quads+=fingerprint(b,digest);b.close();
  }
  String threadDigest=threadOwnership();
  var b=LodQuadBuilder.getBuilder(false,null);for(int r=0;r<2000;r++){batch(b,8);b.close();}Thread.sleep(1000);
  var cpu=ManagementFactory.getThreadMXBean();var allocation=(com.sun.management.ThreadMXBean)cpu;allocation.setThreadAllocatedMemoryEnabled(true);long id=Thread.currentThread().threadId();long ns=0,bytes=0;StringBuilder observations=new StringBuilder("[");
  for(int r=0;r<3;r++){
   for(int i=0;i<500;i++){batch(b,8);b.close();}Thread.sleep(200);
   long start=cpu.getCurrentThreadCpuTime(),before=allocation.getThreadAllocatedBytes(id);
   for(int i=0;i<2000;i++){batch(b,8);sink+=b.getCurrentOpaqueQuadsCount();b.close();}
   long time=cpu.getCurrentThreadCpuTime()-start,allocated=allocation.getThreadAllocatedBytes(id)-before;ns+=time;bytes+=allocated;if(r>0)observations.append(',');observations.append("{\"cpu_ns\":").append(time).append(",\"allocated_bytes\":").append(allocated).append('}');
  }
  observations.append(']');System.out.println("HORIZON_QUAD_RESULT {\"enabled\":"+enabled+",\"applied_hook_checked\":true,\"geometry_digest\":\""+HexFormat.of().formatHex(digest.digest())+"\",\"thread_geometry_digest\":\""+threadDigest+"\",\"thread_workers\":4,\"geometry_quads\":"+quads+",\"input_faces\":"+faces+",\"measured_faces\":3072000,\"checksum\":"+sink+",\"cpu_ns\":"+ns+",\"allocated_bytes\":"+bytes+",\"observations\":"+observations+"}");
 }
}
