package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.NativeImage;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.client.Minecraft;
import net.minecraft.client.color.block.BlockColors;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.particle.*;
import net.minecraft.client.renderer.block.BlockStateModelSet;
import net.minecraft.client.renderer.texture.*;
import net.minecraft.client.resources.metadata.animation.FrameSize;
import net.minecraft.client.resources.model.ModelManager;
import net.minecraft.client.resources.model.sprite.Material;
import net.minecraft.core.*;
import net.minecraft.resources.Identifier;
import net.minecraft.server.Bootstrap;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;

/** Calls the real client mining and destruction methods; records positions at enqueue.
 * Sprite/tint lookup is stubbed. Particle-local RNG and velocities are not compared. */
public final class VanillaBlockParticlesOracle {
    private static final Gson JSON = new Gson();
    private static Object unsafe;
    private static Method allocate;
    private static Field field(Class<?> type, String name) throws Exception {
        var f=type.getDeclaredField(name);f.setAccessible(true);return f;
    }
    private static <T> T make(Class<T> type) throws Exception { return type.cast(allocate.invoke(unsafe,type)); }
    private static final class Level0 extends ClientLevel {
        BlockPos pos;BlockState state;
        Level0() { super(null,null,null,null,0,0,null,false,0L,0); }
        @Override public BlockState getBlockState(BlockPos p) { return p.equals(pos)?state:Blocks.AIR.defaultBlockState(); }
    }
    private static final class Sprite extends TextureAtlasSprite {
        Sprite() { super(TextureAtlas.LOCATION_BLOCKS,new SpriteContents(Identifier.parse("minecraft:missingno"),new FrameSize(16,16),new NativeImage(16,16,true)),16,16,0,0,0); }
    }
    private static final class Models extends BlockStateModelSet {
        private final Material.Baked material=new Material.Baked(new Sprite(),false);
        Models() { super(Map.of(),null); }
        @Override public Material.Baked getParticleMaterial(BlockState state) { return material; }
    }
    private static final class Recorder extends ParticleEngine {
        final List<double[]> positions=new ArrayList<>();
        final Field[] xyz;
        Recorder(ClientLevel level) throws Exception {
            super(level,null);xyz=new Field[]{field(Particle.class,"x"),field(Particle.class,"y"),field(Particle.class,"z")};
        }
        @Override public void add(Particle p) {
            try { positions.add(new double[]{xyz[0].getDouble(p),xyz[1].getDouble(p),xyz[2].getDouble(p)}); }
            catch (Exception e) { throw new RuntimeException(e); }
        }
    }
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();Bootstrap.bootStrap();
        var u=Class.forName("sun.misc.Unsafe");unsafe=field(u,"theUnsafe").get(null);allocate=u.getMethod("allocateInstance",Class.class);
        var mc=make(Minecraft.class);field(Minecraft.class,"instance").set(null,mc);
        var manager=make(ModelManager.class);field(ModelManager.class,"blockStateModelSet").set(manager,new Models());
        field(Minecraft.class,"modelManager").set(mc,manager);field(Minecraft.class,"blockColors").set(mc,new BlockColors());
        var level=make(Level0.class);field(ClientLevel.class,"minecraft").set(level,mc);
        var recorder=new Recorder(level);field(Minecraft.class,"particleEngine").set(mc,recorder);
        var crack=ClientLevel.class.getDeclaredMethod("addBreakingParticles",BlockPos.class,Direction.class,BlockState.class);crack.setAccessible(true);
        try (var output=Files.newBufferedWriter(Path.of(args[1]))) {
            for (String line:Files.readAllLines(Path.of(args[0]))) {
                var row=JsonParser.parseString(line).getAsJsonObject();var p=row.getAsJsonArray("pos");
                level.pos=new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());level.state=Block.stateById(row.get("state").getAsInt());
                var hits=new ArrayList<Object>();var next=new ArrayList<Long>();
                for (var face:Direction.values()) {
                    var random=RandomSource.create(row.get("seed").getAsLong());field(Level.class,"random").set(level,random);recorder.positions.clear();
                    if (!level.state.getShape(level,level.pos).isEmpty()) crack.invoke(level,level.pos,face,level.state);
                    hits.add(recorder.positions.isEmpty()?null:recorder.positions.getFirst());next.add(random.nextLong());
                    if (recorder.positions.size()>1) throw new AssertionError("multiple mining particles");
                }
                recorder.positions.clear();level.addDestroyBlockEffect(level.pos,level.state);
                output.write(JSON.toJson(Map.of("case",row.get("case").getAsInt(),"hits",hits,"next",next,"destroy",recorder.positions)));output.newLine();
            }
        }
    }
}
