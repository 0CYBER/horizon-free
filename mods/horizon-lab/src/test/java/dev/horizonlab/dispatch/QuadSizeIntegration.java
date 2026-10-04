package dev.horizonlab.dispatch;
import com.seibel.distanthorizons.core.dataObjects.render.bufferBuilding.LodQuadBuilder;
import com.seibel.distanthorizons.core.dataObjects.render.bufferBuilding.BufferQuad;
import com.seibel.distanthorizons.core.enums.EDhDirection;
import java.lang.management.ManagementFactory;
import com.sun.management.HotSpotDiagnosticMXBean;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLifecycleEvents;
public final class QuadSizeIntegration implements ClientModInitializer {
 static long size(Object value)throws Exception { return ((Long)ClassLoader.getSystemClassLoader().loadClass("dev.horizonlab.metrics.ObjectSizeAgent").getMethod("size",Object.class).invoke(null,value)).longValue(); }
 public void onInitializeClient() { ClientLifecycleEvents.CLIENT_STARTED.register(client -> {
  try {
   var b=LodQuadBuilder.getBuilder(false,null);
   for(int x=0;x<4;x++) b.addQuadAdj(EDhDirection.NORTH,(short)x,(short)0,(short)0,(short)1,(short)1,0xff113355,(short)3,(byte)2,(byte)15,(byte)4);
   long scratch=0;boolean hook=false;
   for(var f:LodQuadBuilder.class.getDeclaredFields())if(f.getName().contains("horizon$discardedQuad")) { hook=true;f.setAccessible(true);Object v=f.get(b);if(v==null)throw new AssertionError("Missing merged scratch");scratch=size(v); }
   boolean headers=Boolean.parseBoolean(ManagementFactory.getPlatformMXBean(HotSpotDiagnosticMXBean.class).getVMOption("UseCompactObjectHeaders").getValue());
   var vm=ManagementFactory.getPlatformMXBean(HotSpotDiagnosticMXBean.class);
   var result=new java.util.LinkedHashMap<String,Object>();
   result.put("enabled",hook);result.put("compact_headers",headers);
   result.put("builder_shallow_bytes",size(b));result.put("quad_shallow_bytes",size(new BufferQuad()));
   result.put("pending_scratch_shallow_bytes",scratch);result.put("geometry_quads",b.getCurrentOpaqueQuadsCount());
   result.put("java_vendor",System.getProperty("java.vendor"));result.put("java_runtime_version",System.getProperty("java.runtime.version"));
   result.put("object_alignment_bytes",Integer.parseInt(vm.getVMOption("ObjectAlignmentInBytes").getValue()));
   result.put("compressed_oops",Boolean.parseBoolean(vm.getVMOption("UseCompressedOops").getValue()));
   result.put("compressed_class_pointers",Boolean.parseBoolean(vm.getVMOption("UseCompressedClassPointers").getValue()));
   System.out.println("HORIZON_SIZE_RESULT "+new com.google.gson.Gson().toJson(result));
   b.close();System.exit(0);
  } catch(Exception e) { throw new AssertionError("Private size probe failed",e); }
 }); }
}
