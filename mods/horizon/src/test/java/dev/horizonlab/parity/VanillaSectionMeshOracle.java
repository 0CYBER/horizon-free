package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.blaze3d.vertex.QuadInstance;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.CompletableFuture;
import java.util.function.Function;
import net.minecraft.client.color.block.BlockColors;
import net.minecraft.client.renderer.BiomeColors;
import net.minecraft.client.renderer.block.*;
import net.minecraft.client.renderer.block.dispatch.*;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.texture.*;
import net.minecraft.client.resources.metadata.animation.FrameSize;
import net.minecraft.client.resources.model.*;
import net.minecraft.client.resources.model.cuboid.*;
import net.minecraft.client.resources.model.geometry.BakedQuad;
import net.minecraft.client.resources.model.sprite.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.util.StrictJsonParser;
import net.minecraft.world.level.CardinalLighting;
import net.minecraft.world.level.ColorResolver;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.level.DryFoliageColor;
import net.minecraft.world.level.FoliageColor;
import net.minecraft.world.level.GrassColor;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeManager;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceKey;
import net.minecraft.util.ARGB;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.lighting.LevelLightEngine;
import net.minecraft.world.level.material.FluidState;
import org.joml.Vector3fc;
import static dev.horizonlab.parity.VanillaCuboidModelOracle.map;

/** Runs the client's own ModelBlockRenderer (smooth lighting on) and FluidRenderer
 * over small fixed scenes, the way SectionCompiler visits one section, and
 * records every emitted vertex. Block models are baked from the jar's own
 * blockstate and model resources; every texture becomes a real sprite alone in
 * a unit atlas so UVs stay in sprite-local [0,1] space. Fluid sprites share one
 * strip with gaps so their vertex stream still identifies the sprite. */
public final class VanillaSectionMeshOracle {
    private static final Gson JSON = new GsonBuilder().serializeNulls().create();
    private static final String[] FLUID_SPRITES = {"minecraft:block/water_still", "minecraft:block/water_flow",
        "minecraft:block/water_overlay", "minecraft:block/lava_still", "minecraft:block/lava_flow"};
    private static final int STRIP = 2 * FLUID_SPRITES.length;

    private static final class Sprite extends TextureAtlasSprite {
        Sprite(SpriteContents contents, int slots, int slot) {
            super(TextureAtlas.LOCATION_BLOCKS, contents, contents.width() * slots, contents.height(), contents.width() * slot, 0, 0);
        }
    }
    private static final Map<Identifier, TextureAtlasSprite> SPRITES = new HashMap<>();
    private static SpriteContents contents(Identifier id) throws IOException {
        var path = "/assets/" + id.getNamespace() + "/textures/" + id.getPath() + ".png";
        try (var input = VanillaSectionMeshOracle.class.getResourceAsStream(path)) {
            NativeImage image = input == null ? new NativeImage(16, 16, true) : NativeImage.read(input);
            int size = Math.min(image.getWidth(), image.getHeight());
            return new SpriteContents(id, new FrameSize(image.getWidth(), size), image);
        }
    }
    private static TextureAtlasSprite sprite(Identifier id) {
        return SPRITES.computeIfAbsent(id, key -> {
            try {
                int slot = Arrays.asList(FLUID_SPRITES).indexOf(key.toString());
                return slot < 0 ? new Sprite(contents(key), 1, 0) : new Sprite(contents(key), STRIP, 2 * slot);
            } catch (IOException error) { throw new UncheckedIOException(error); }
        });
    }
    /** Hands out the unit-atlas sprite named by each material. */
    private static final class Materials extends MaterialBaker {
        Materials(SpriteLoader.Preparations atlas) { super(atlas, atlas); }
        @Override public Material.Baked get(Material material, ModelDebugName name) {
            return new Material.Baked(sprite(material.sprite()), material.forceTranslucent());
        }
    }
    private static final class Baker implements ModelBaker {
        final Map<Identifier, ResolvedModel> resolved; final ResolvedModel missing; final MaterialBaker materials;
        final Map<SharedOperationKey<?>, Object> cache = new HashMap<>();
        Baker(Map<Identifier, ResolvedModel> resolved, ResolvedModel missing, MaterialBaker materials) {
            this.resolved = resolved; this.missing = missing; this.materials = materials;
        }
        public ResolvedModel getModel(Identifier id) { return resolved.getOrDefault(id, missing); }
        public BlockStateModelPart missingBlockModelPart() { throw new AssertionError("missing block model part"); }
        public MaterialBaker materials() { return materials; }
        public ModelBaker.Interner interner() {
            return new ModelBaker.Interner() {
                public Vector3fc vector(Vector3fc vector) { return vector; }
                public BakedQuad.MaterialInfo materialInfo(BakedQuad.MaterialInfo value) { return value; }
            };
        }
        @SuppressWarnings("unchecked")
        public <T> T compute(SharedOperationKey<T> key) {
            var value = cache.get(key);
            if (value == null) { value = key.compute(this); cache.put(key, value); }
            return (T) value;
        }
    }
    private static int[] colormap(String name) throws IOException {
        try (var input = VanillaSectionMeshOracle.class.getResourceAsStream("/assets/minecraft/textures/colormap/" + name + ".png");
             var image = NativeImage.read(input)) {
            return image.makePixelArray();
        }
    }
    private static String resource(String path) throws IOException {
        try (var input = VanillaSectionMeshOracle.class.getResourceAsStream(path)) {
            return input == null ? null : new String(input.readAllBytes(), StandardCharsets.UTF_8);
        }
    }
    /** Real blockstate definitions and models for the requested blocks. */
    private static final class Models {
        final Map<BlockState, BlockStateModel.UnbakedRoot> roots = new HashMap<>();
        final Map<BlockState, BlockStateModel> baked = new HashMap<>();
        final Baker baker;
        Models(Set<Block> blocks, MaterialBaker materials) throws IOException {
            var unbaked = new HashMap<Identifier, UnbakedModel>();
            for (var block : blocks) {
                var id = BuiltInRegistries.BLOCK.getKey(block);
                var raw = resource("/assets/" + id.getNamespace() + "/blockstates/" + id.getPath() + ".json");
                if (raw == null) continue;
                var definition = BlockStateModelDispatcher.CODEC.parse(JsonOps.INSTANCE, StrictJsonParser.parse(raw)).getOrThrow();
                roots.putAll(definition.instantiate(block.getStateDefinition(), id::toString));
            }
            var pending = new ArrayList<Identifier>();
            for (var root : new HashSet<>(roots.values())) root.resolveDependencies(pending::add);
            for (int i = 0; i < pending.size(); i++) {
                var id = pending.get(i);
                if (unbaked.containsKey(id)) continue;
                var raw = resource("/assets/" + id.getNamespace() + "/models/" + id.getPath() + ".json");
                if (raw == null) continue;
                var model = CuboidModel.fromStream(new StringReader(raw));
                unbaked.put(id, model);
                if (model.parent() != null) pending.add(model.parent());
            }
            var discovery = new ModelDiscovery(unbaked, MissingCuboidModel.missingModel());
            discovery.addRoot(resolver -> { for (var id : unbaked.keySet()) resolver.markDependency(id); });
            var resolved = discovery.resolve();
            baker = new Baker(resolved, discovery.missingModel(), materials);
        }
        BlockStateModel get(BlockState state) {
            return baked.computeIfAbsent(state, s -> Objects.requireNonNull(roots.get(s), () -> "no model for " + s).bake(s, baker));
        }
    }
    /** ClientLevel.calculateBlockTint over a BiomeManager with a fixed noise-biome map:
     * quart cells listed in the scene, a default elsewhere in the 3x3 chunk region and
     * plains outside it (chunks that are not loaded). */
    private static final class TintBiomes {
        final BiomeManager manager; final int radius;
        final Map<Long, Holder<Biome>> cells = new HashMap<>();
        final Holder<Biome> dflt, plains;
        TintBiomes(JsonObject o, HolderLookup.RegistryLookup<Biome> lookup) {
            radius = o.get("radius").getAsInt();
            Function<String, Holder<Biome>> named = name -> lookup.getOrThrow(ResourceKey.create(Registries.BIOME, Identifier.parse(name)));
            plains = named.apply("minecraft:plains");
            dflt = named.apply(o.get("default").getAsString());
            for (var item : o.getAsJsonArray("cells")) {
                var c = item.getAsJsonArray();
                cells.put(BlockPos.asLong(c.get(0).getAsInt(), c.get(1).getAsInt(), c.get(2).getAsInt()), named.apply(c.get(3).getAsString()));
            }
            manager = new BiomeManager((qx, qy, qz) -> {
                if (qx < -4 || qx >= 8 || qz < -4 || qz >= 8) return plains;
                return cells.getOrDefault(BlockPos.asLong(qx, Math.max(-16, Math.min(79, qy)), qz), dflt);
            }, o.get("seed").getAsLong());
        }
        int tint(BlockPos pos, ColorResolver resolver) {
            if (radius == 0) return resolver.getColor(manager.getBiome(pos).value(), pos.getX(), pos.getZ());
            int count = (radius * 2 + 1) * (radius * 2 + 1);
            int red = 0, green = 0, blue = 0;
            var cursor = new Cursor3D(pos.getX() - radius, pos.getY(), pos.getZ() - radius, pos.getX() + radius, pos.getY(), pos.getZ() + radius);
            var next = new BlockPos.MutableBlockPos();
            while (cursor.advance()) {
                next.set(cursor.nextX(), cursor.nextY(), cursor.nextZ());
                int color = resolver.getColor(manager.getBiome(next).value(), next.getX(), next.getZ());
                red += ARGB.red(color); green += ARGB.green(color); blue += ARGB.blue(color);
            }
            return ARGB.color(red / count, green / count, blue / count);
        }
    }
    private static long key(int x, int y, int z) { return BlockPos.asLong(x, y, z); }
    /** A fixed region: listed blocks over air, listed light over a default. */
    private static final class Scene implements BlockAndTintGetter {
        final Map<Long, BlockState> blocks = new HashMap<>();
        final Map<Long, int[]> light = new HashMap<>();
        int[] defaultLight; boolean nether; int grass, foliage, dry, water; TintBiomes biomes;
        public CardinalLighting cardinalLighting() { return nether ? CardinalLighting.NETHER : CardinalLighting.DEFAULT; }
        public LevelLightEngine getLightEngine() { return LevelLightEngine.EMPTY; }
        public int getBrightness(LightLayer layer, BlockPos pos) {
            var value = light.getOrDefault(pos.asLong(), defaultLight);
            return layer == LightLayer.SKY ? value[0] : value[1];
        }
        public int getBlockTint(BlockPos pos, ColorResolver resolver) {
            if (biomes != null) return biomes.tint(pos, resolver);
            int rgb = resolver == BiomeColors.GRASS_COLOR_RESOLVER ? grass : resolver == BiomeColors.FOLIAGE_COLOR_RESOLVER ? foliage
                : resolver == BiomeColors.DRY_FOLIAGE_COLOR_RESOLVER ? dry : resolver == BiomeColors.WATER_COLOR_RESOLVER ? water : -1;
            if (rgb == -1) throw new AssertionError("unexpected colour resolver");
            return 0xFF000000 | rgb;
        }
        public BlockEntity getBlockEntity(BlockPos pos) { return null; }
        public BlockState getBlockState(BlockPos pos) { return blocks.getOrDefault(pos.asLong(), Blocks.AIR.defaultBlockState()); }
        public FluidState getFluidState(BlockPos pos) { return getBlockState(pos).getFluidState(); }
        public int getHeight() { return 384; }
        public int getMinY() { return -64; }
    }
    private static final class Recorder {
        final List<Object> quads = new ArrayList<>();
        List<Object> pending; boolean forceSolid; BlockPos block; String kind, layer, sprite;
        void vertex(float x, float y, float z, int color, float u, float v, int light) {
            if (pending == null) pending = new ArrayList<>();
            pending.add(new Object[]{x, y, z, u, v, color, light});
            if (pending.size() == 4) {
                quads.add(map("block", new int[]{block.getX(), block.getY(), block.getZ()}, "kind", kind, "layer", layer, "sprite", sprite, "vertices", pending));
                pending = null;
            }
        }
    }
    private static String layer(ChunkSectionLayer layer) { return layer.name().toLowerCase(Locale.ROOT); }
    private static Object scene(JsonObject row, Models models, FluidStateModelSet fluids, HolderLookup.RegistryLookup<Biome> biomeLookup) {
        var scene = new Scene();
        var light = row.getAsJsonArray("light");
        scene.defaultLight = new int[]{light.get(0).getAsInt(), light.get(1).getAsInt()};
        scene.nether = row.get("nether").getAsBoolean();
        var colors = row.getAsJsonObject("colors");
        scene.grass = colors.get("grass").getAsInt(); scene.foliage = colors.get("foliage").getAsInt();
        scene.dry = colors.get("dry_foliage").getAsInt(); scene.water = colors.get("water").getAsInt();
        if (row.has("biomes")) scene.biomes = new TintBiomes(row.getAsJsonObject("biomes"), biomeLookup);
        for (var item : row.getAsJsonArray("blocks")) {
            var b = item.getAsJsonArray();
            scene.blocks.put(key(b.get(0).getAsInt(), b.get(1).getAsInt(), b.get(2).getAsInt()), Block.stateById(b.get(3).getAsInt()));
        }
        for (var item : row.getAsJsonArray("lights")) {
            var l = item.getAsJsonArray();
            scene.light.put(key(l.get(0).getAsInt(), l.get(1).getAsInt(), l.get(2).getAsInt()), new int[]{l.get(3).getAsInt(), l.get(4).getAsInt()});
        }
        var recorder = new Recorder();
        BlockModelLighter.enableCaching();
        boolean smooth = !row.has("smooth") || row.get("smooth").getAsBoolean();
        boolean cutoutLeaves = !row.has("cutout_leaves") || row.get("cutout_leaves").getAsBoolean();
        LeavesBlock.setCutoutLeaves(cutoutLeaves);
        var renderer = new ModelBlockRenderer(smooth, true, BlockColors.createDefault());
        var fluidRenderer = new FluidRenderer(fluids);
        BlockQuadOutput output = (x, y, z, quad, instance) -> {
            recorder.kind = "model"; recorder.layer = recorder.forceSolid ? "solid" : layer(quad.materialInfo().layer());
            recorder.sprite = quad.materialInfo().sprite().contents().name().toString();
            int emission = quad.materialInfo().lightEmission();
            int slot = Arrays.asList(FLUID_SPRITES).indexOf(recorder.sprite);
            for (int i = 0; i < 4; i++) {
                var p = quad.position(i); long uv = quad.packedUV(i);
                float u = Float.intBitsToFloat((int) (uv >>> 32));
                recorder.vertex(p.x() + x, p.y() + y, p.z() + z, instance.getColor(i),
                    slot < 0 ? u : u * STRIP - 2 * slot, Float.intBitsToFloat((int) uv), instance.getLightCoordsWithEmission(i, emission));
            }
        };
        int baseY = row.get("section_y").getAsInt() * 16;
        for (var pos : BlockPos.betweenClosed(0, baseY, 0, 15, baseY + 15, 15)) {
            var state = scene.getBlockState(pos);
            if (state.isAir()) continue;
            recorder.block = pos.immutable();
            var fluid = state.getFluidState();
            if (!fluid.isEmpty()) {
                fluidRenderer.tesselate(scene, pos, sectionLayer -> new VertexConsumer() {
                    public void addVertex(float x, float y, float z, int color, float u, float v, int overlay, int light, float nx, float ny, float nz) {
                        int slot = (int) Math.floor(u * STRIP / 2.0f);
                        recorder.kind = "fluid"; recorder.layer = layer(sectionLayer); recorder.sprite = FLUID_SPRITES[slot];
                        recorder.vertex(x, y, z, color, u * STRIP - 2 * slot, v, light);
                    }
                    public VertexConsumer addVertex(float x, float y, float z) { throw new AssertionError(); }
                    public VertexConsumer setColor(int r, int g, int b, int a) { throw new AssertionError(); }
                    public VertexConsumer setColor(int color) { throw new AssertionError(); }
                    public VertexConsumer setUv(float u, float v) { throw new AssertionError(); }
                    public VertexConsumer setUv1(int u, int v) { throw new AssertionError(); }
                    public VertexConsumer setUv2(int u, int v) { throw new AssertionError(); }
                    public VertexConsumer setUv3(float u, float v) { throw new AssertionError(); }
                    public VertexConsumer setNormal(float x, float y, float z) { throw new AssertionError(); }
                    public VertexConsumer setLineWidth(float width) { throw new AssertionError(); }
                }, state, fluid);
            }
            if (state.getRenderShape() == RenderShape.MODEL) {
                recorder.forceSolid = ModelBlockRenderer.forceOpaque(cutoutLeaves, state);
                renderer.tesselateBlock(output, SectionPos.sectionRelative(pos.getX()), SectionPos.sectionRelative(pos.getY()), SectionPos.sectionRelative(pos.getZ()),
                    scene, pos, state, models.get(state), state.getSeed(pos));
            }
        }
        BlockModelLighter.clearCache();
        if (recorder.pending != null) throw new AssertionError("partial quad");
        return map("name", row.get("name").getAsString(), "quads", recorder.quads);
    }
    private static BlockColors COLORS;
    private static HolderLookup.RegistryLookup<Biome> biomeLookup;
    /** Per-state facts the Rust mesher derives from prepared resources. */
    private static Object facts(BlockState state, int id) {
        var level = EmptyBlockGetter.INSTANCE;
        var offsets = new ArrayList<Object>();
        if (state.hasOffsetFunction()) {
            for (var p : new BlockPos[]{new BlockPos(0, 0, 0), new BlockPos(5, 7, -3), new BlockPos(-17, 70, 31), new BlockPos(123, -40, -456), new BlockPos(9, 1, 2)}) {
                var o = state.getOffset(p);
                offsets.add(new double[]{o.x, o.y, o.z});
            }
        }
        var block = state.getBlock();
        return map("id", id, "shade", state.getShadeBrightness(level, BlockPos.ZERO), "emissive", state.emissiveRendering(),
            "legacy_solid", state.isSolid(), "light_permeable", state.isLightPermeable(), "solid_render", state.isSolidRender(),
            "full_collision", state.isCollisionShapeFullBlock(level, BlockPos.ZERO), "emission", state.getLightEmission(),
            "overlay", block instanceof HalfTransparentBlock || block instanceof LeavesBlock,
            "tint_sources", COLORS.getTintSources(state).size(), "offsets", offsets);
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(Path.of(args[2]).resolve("block-tags.json"))).getAsJsonObject());
        LeavesBlock.setCutoutLeaves(true);
        GrassColor.init(colormap("grass")); FoliageColor.init(colormap("foliage")); DryFoliageColor.init(colormap("dry_foliage"));
        COLORS = BlockColors.createDefault();
        if (args[0].equals("facts")) {
            try (var output = Files.newBufferedWriter(Path.of(args[1]))) {
                int id = 0;
                for (var state : Block.BLOCK_STATE_REGISTRY) {
                    if (Block.getId(state) != id) throw new AssertionError("state registry order");
                    output.write(JSON.toJson(facts(state, id++))); output.newLine();
                }
            }
            return;
        }
        var rows = new ArrayList<JsonObject>();
        try (var input = Files.newBufferedReader(Path.of(args[0]))) {
            for (String line; (line = input.readLine()) != null; ) rows.add(JsonParser.parseString(line).getAsJsonObject());
        }
        var blocks = new HashSet<Block>();
        for (var row : rows) for (var item : row.getAsJsonArray("blocks")) blocks.add(Block.stateById(item.getAsJsonArray().get(3).getAsInt()).getBlock());
        var missingContents = new SpriteContents(Identifier.parse("minecraft:missingno"), new FrameSize(16, 16), new NativeImage(16, 16, true));
        var missing = new Sprite(missingContents, 1, 0);
        var atlas = new SpriteLoader.Preparations(16, 16, 0, missing, Map.of(), CompletableFuture.completedFuture(null));
        var materials = new Materials(atlas);
        var models = new Models(blocks, materials);
        biomeLookup = net.minecraft.data.registries.VanillaRegistries.createWorldLookup().lookupOrThrow(Registries.BIOME);
        var fluidModels = FluidStateModelSet.bake(materials);
        var fluids = new FluidStateModelSet(fluidModels, fluidModels.values().iterator().next());
        try (var output = Files.newBufferedWriter(Path.of(args[1]))) {
            for (var row : rows) { output.write(JSON.toJson(scene(row, models, fluids, biomeLookup))); output.newLine(); }
        }
    }
}
