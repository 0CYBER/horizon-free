package dev.horizonlab.parity;

/** Original export of block identifiers, used solely as tag lookup inputs. */
public final class VanillaTagExport {
    public static void main(String[] paths) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var names=new java.util.TreeMap<String,String>();
        for(var id:net.minecraft.core.registries.BuiltInRegistries.BLOCK.keySet())names.put(id.toString(),id.toString());
        java.nio.file.Files.writeString(java.nio.file.Path.of(paths[0]),new com.google.gson.Gson().toJson(names),java.nio.charset.StandardCharsets.UTF_8);
    }
}
