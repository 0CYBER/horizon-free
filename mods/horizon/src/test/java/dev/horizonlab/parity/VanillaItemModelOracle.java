package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.serialization.Codec;
import com.mojang.serialization.JsonOps;
import com.mojang.serialization.MapCodec;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.CompletableFuture;
import java.util.stream.*;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.model.geom.EntityModelSet;
import net.minecraft.client.renderer.item.*;
import net.minecraft.client.renderer.item.properties.select.SelectItemModelProperty;
import net.minecraft.client.renderer.special.SpecialModelRenderer;
import net.minecraft.client.renderer.special.SpecialModelRenderers;
import net.minecraft.client.renderer.texture.*;
import net.minecraft.client.resources.metadata.animation.AnimationMetadataSection;
import net.minecraft.client.resources.model.*;
import net.minecraft.client.resources.model.cuboid.*;
import net.minecraft.client.resources.model.geometry.BakedQuad;
import net.minecraft.client.resources.model.sprite.SpriteGetter;
import net.minecraft.client.resources.model.sprite.SpriteId;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryOps;
import net.minecraft.util.StrictJsonParser;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.ItemOwner;
import net.minecraft.world.entity.HumanoidArm;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.minecraft.world.item.*;
import net.minecraft.world.item.component.BundleContents;
import net.minecraft.world.level.GrassColor;
import net.minecraft.world.level.Level;
import org.joml.Matrix4f;
import org.joml.Vector3fc;

/**
 * Drives the pinned client's own item pipeline headlessly: the ClientItem codec,
 * ModelDiscovery, ModelBakery with real baked item models, ItemModelResolver
 * semantics and ItemStackRenderState layers. Sprites are real PNGs in a unit
 * atlas; the player skin cache and GPU are never touched.
 */
public final class VanillaItemModelOracle {
    private static final Gson JSON = new GsonBuilder().serializeNulls().create();
    private static long bits(float value) { return Integer.toUnsignedLong(Float.floatToIntBits(value)); }
    static Map<String, Object> map(Object... values) {
        var result = new TreeMap<String, Object>();
        for (int i = 0; i < values.length; i += 2) result.put((String) values[i], values[i + 1]);
        return result;
    }

    static Path root;
    static HolderLookup.Provider registries;
    static RegistryOps<JsonElement> ops;
    static String frozenTime = "";
    static final Identifier BLOCKS = TextureAtlas.LOCATION_BLOCKS, ITEMS = TextureAtlas.LOCATION_ITEMS;

    static final class Sprite extends TextureAtlasSprite {
        Sprite(Identifier atlas, SpriteContents contents) { super(atlas, contents, contents.width(), contents.height(), 0, 0, 0); }
    }

    /** Sprites loaded on demand from the extracted jar. */
    static final class Regions extends HashMap<Identifier, TextureAtlasSprite> {
        final boolean items;
        Regions(boolean items) { this.items = items; }
        @Override public TextureAtlasSprite get(Object key) {
            var id = (Identifier) key;
            boolean itemAtlas = id.getPath().startsWith("item/") || id.getPath().startsWith("trims/");
            if (itemAtlas != items) return null;
            return computeIfAbsent(id, k -> load(k, items ? ITEMS : BLOCKS));
        }
    }
    static TextureAtlasSprite load(Identifier id, Identifier atlas) {
        try {
            var png = root.resolve("assets").resolve(id.getNamespace()).resolve("textures").resolve(id.getPath() + ".png");
            if (!Files.isRegularFile(png)) {
                // Armour trim textures come from the atlas's paletted_permutations source.
                var generated = generatedSprites(atlas).get(id);
                return generated == null ? null : new Sprite(atlas, generated.get(null));
            }
            NativeImage image;
            try (var in = Files.newInputStream(png)) { image = NativeImage.read(in); }
            var mcmeta = png.resolveSibling(png.getFileName() + ".mcmeta");
            Optional<AnimationMetadataSection> animation = Optional.empty();
            if (Files.isRegularFile(mcmeta)) {
                var json = JsonParser.parseString(Files.readString(mcmeta)).getAsJsonObject();
                if (json.has("animation"))
                    animation = Optional.of(AnimationMetadataSection.CODEC.parse(JsonOps.INSTANCE, json.get("animation")).getOrThrow());
            }
            var size = animation.map(a -> a.calculateFrameSize(image.getWidth(), image.getHeight()))
                .orElse(new net.minecraft.client.resources.metadata.animation.FrameSize(image.getWidth(), image.getHeight()));
            return new Sprite(atlas, new SpriteContents(id, size, image, animation, List.of(), Optional.empty()));
        } catch (IOException e) { throw new UncheckedIOException(e); }
    }

    static final Map<Identifier, Map<Identifier, net.minecraft.client.renderer.texture.atlas.SpriteSource.DiscardableLoader>> GENERATED = new HashMap<>();
    /** The atlas's paletted_permutations sprites, run by vanilla's own source over the extracted resources. */
    static Map<Identifier, net.minecraft.client.renderer.texture.atlas.SpriteSource.DiscardableLoader> generatedSprites(Identifier atlas) {
        return GENERATED.computeIfAbsent(atlas, a -> {
            Map<Identifier, net.minecraft.client.renderer.texture.atlas.SpriteSource.DiscardableLoader> out = new HashMap<>();
            var file = "/assets/" + a.getNamespace() + "/atlases/" + a.getPath().replaceFirst("^textures/atlas/", "").replaceFirst("\\.png$", "") + ".json";
            try (var in = VanillaItemModelOracle.class.getResourceAsStream(file)) {
                if (in == null) return out;
                var json = JsonParser.parseString(new String(in.readAllBytes(), java.nio.charset.StandardCharsets.UTF_8));
                var sources = net.minecraft.client.renderer.texture.atlas.SpriteSources.FILE_CODEC.parse(JsonOps.INSTANCE, json).getOrThrow();
                var output = new net.minecraft.client.renderer.texture.atlas.SpriteSource.Output() {
                    @Override public void add(Identifier id, net.minecraft.client.renderer.texture.atlas.SpriteSource.DiscardableLoader sprite) { out.put(id, sprite); }
                    @Override public void removeAll(java.util.function.Predicate<Identifier> predicate) { out.keySet().removeIf(predicate); }
                };
                for (var source : sources)
                    if (source instanceof net.minecraft.client.renderer.texture.atlas.sources.PalettedPermutations) source.run(FILES, output);
            } catch (IOException e) { throw new UncheckedIOException(e); }
            return out;
        });
    }
    /** The extracted resources as a single-pack resource manager. */
    static final net.minecraft.server.packs.resources.ResourceManager FILES = new net.minecraft.server.packs.resources.ResourceManager() {
        Path path(Identifier id) { return root.resolve("assets").resolve(id.getNamespace()).resolve(id.getPath()); }
        @Override public Optional<net.minecraft.server.packs.resources.Resource> getResource(Identifier id) {
            var file = path(id);
            return Files.isRegularFile(file) ? Optional.of(new net.minecraft.server.packs.resources.Resource(null, () -> Files.newInputStream(file))) : Optional.empty();
        }
        @Override public java.util.Set<String> getNamespaces() { return java.util.Set.of("minecraft"); }
        @Override public List<net.minecraft.server.packs.resources.Resource> getResourceStack(Identifier id) { return getResource(id).stream().toList(); }
        @Override public Map<Identifier, net.minecraft.server.packs.resources.Resource> listResources(String directory, net.minecraft.server.packs.resources.ResourceManager.Selector selector) { throw new UnsupportedOperationException(); }
        @Override public Map<Identifier, List<net.minecraft.server.packs.resources.Resource>> listResourceStacks(String directory, net.minecraft.server.packs.resources.ResourceManager.Selector selector) { throw new UnsupportedOperationException(); }
        @Override public java.util.stream.Stream<net.minecraft.server.packs.PackResources> listPacks() { return java.util.stream.Stream.empty(); }
    };

    // --- Tagging of special renderers so the layer carries its codec form. ---
    static final class Tagged implements SpecialModelRenderer.Unbaked<Object> {
        final SpecialModelRenderer.Unbaked<?> delegate;
        Tagged(SpecialModelRenderer.Unbaked<?> delegate) { this.delegate = delegate; }
        @SuppressWarnings({"unchecked", "rawtypes"})
        public MapCodec<? extends SpecialModelRenderer.Unbaked<Object>> type() { return (MapCodec) delegate.type(); }
        public SpecialModelRenderer<Object> bake(SpecialModelRenderer.BakingContext context) {
            var baked = delegate.bake(context);
            if (baked == null) return null;
            @SuppressWarnings({"unchecked", "rawtypes"}) JsonElement config = SpecialModelRenderers.CODEC.encodeStart(JsonOps.INSTANCE, (SpecialModelRenderer.Unbaked) delegate).getOrThrow();
            return new TaggedRenderer(baked, config);
        }
    }
    static final class TaggedRenderer implements SpecialModelRenderer<Object> {
        final SpecialModelRenderer<?> delegate; final JsonElement config;
        TaggedRenderer(SpecialModelRenderer<?> delegate, JsonElement config) { this.delegate = delegate; this.config = config; }
        public void submit(Object argument, com.mojang.blaze3d.vertex.PoseStack poseStack, net.minecraft.client.renderer.SubmitNodeCollector collector, int light, int overlay, boolean foil, int outline) {}
        public void getExtents(java.util.function.Consumer<Vector3fc> output) { delegate.getExtents(output); }
        public Object extractArgument(ItemStack stack) { return new Argument(argument(config.getAsJsonObject().get("type").getAsString(), stack)); }
    }
    record Argument(JsonElement json) {}
    static JsonElement encode(DataComponentType<?> type, ItemStack stack) {
        var value = stack.get((DataComponentType) type);
        if (value == null) return JsonNull.INSTANCE;
        @SuppressWarnings({"unchecked", "rawtypes"}) var result = ((Codec) type.codecOrThrow()).encodeStart(ops, value).getOrThrow();
        return (JsonElement) result;
    }
    static JsonElement argument(String type, ItemStack stack) {
        return switch (type) {
            case "minecraft:banner" -> encode(DataComponents.BANNER_PATTERNS, stack);
            case "minecraft:decorated_pot" -> encode(DataComponents.POT_DECORATIONS, stack);
            case "minecraft:player_head" -> encode(DataComponents.PROFILE, stack);
            case "minecraft:shield" -> {
                var o = new JsonObject();
                var patterns = encode(DataComponents.BANNER_PATTERNS, stack);
                if (!patterns.isJsonNull()) o.add("minecraft:banner_patterns", patterns);
                var color = encode(DataComponents.BASE_COLOR, stack);
                if (!color.isJsonNull()) o.add("minecraft:base_color", color);
                yield o;
            }
            default -> JsonNull.INSTANCE;
        };
    }
    /** Stands in for LocalTime so a case can script the formatted time. */
    record FrozenTime(Codec<String> valueCodec) implements SelectItemModelProperty<String> {
        public String get(ItemStack s, ClientLevel l, net.minecraft.world.entity.LivingEntity o, int seed, ItemDisplayContext d) { return frozenTime; }
        @SuppressWarnings({"unchecked", "rawtypes"})
        public SelectItemModelProperty.Type type() { return net.minecraft.client.renderer.item.properties.select.LocalTime.TYPE; }
    }
    @SuppressWarnings({"unchecked", "rawtypes"})
    static ItemModel.Unbaked tag(ItemModel.Unbaked u) {
        if (u instanceof SpecialModelWrapper.Unbaked s)
            return new SpecialModelWrapper.Unbaked(s.base(), s.transformation(), new Tagged(s.specialModel()));
        if (u instanceof CompositeModel.Unbaked c)
            return new CompositeModel.Unbaked(c.models().stream().map(VanillaItemModelOracle::tag).collect(Collectors.toList()), c.transformation());
        if (u instanceof ConditionalItemModel.Unbaked c)
            return new ConditionalItemModel.Unbaked(c.transformation(), c.property(), tag(c.onTrue()), tag(c.onFalse()));
        if (u instanceof SelectItemModel.Unbaked s) {
            var sw = (SelectItemModel.UnbakedSwitch) s.unbakedSwitch();
            var cases = new ArrayList<SelectItemModel.SwitchCase>();
            for (var c : (List<SelectItemModel.SwitchCase>) sw.cases()) cases.add(new SelectItemModel.SwitchCase(c.values(), tag(c.model())));
            Object property = sw.property();
            if (property instanceof net.minecraft.client.renderer.item.properties.select.LocalTime) property = new FrozenTime(Codec.STRING);
            return new SelectItemModel.Unbaked(s.transformation(), new SelectItemModel.UnbakedSwitch((SelectItemModelProperty) property, cases), s.fallback().map(VanillaItemModelOracle::tag));
        }
        if (u instanceof RangeSelectItemModel.Unbaked r) {
            var entries = new ArrayList<RangeSelectItemModel.Entry>();
            for (var e : r.entries()) entries.add(new RangeSelectItemModel.Entry(e.threshold(), tag(e.model())));
            return new RangeSelectItemModel.Unbaked(r.transformation(), r.property(), r.scale(), entries, r.fallback().map(VanillaItemModelOracle::tag));
        }
        return u;
    }

    // --- Scripted owner ---
    static final class Actor extends ArmorStand {
        boolean using; int remaining; ItemStack stack = ItemStack.EMPTY; HumanoidArm arm = HumanoidArm.RIGHT; EntityType<?> kind = EntityTypes.PLAYER;
        Actor() { super(null, null); }
        @Override public boolean isUsingItem() { return using; }
        @Override public ItemStack getUseItem() { return using ? stack : ItemStack.EMPTY; }
        @Override public int getUseItemRemainingTicks() { return remaining; }
        @Override public HumanoidArm getMainArm() { return arm; }
        @Override public net.minecraft.core.Holder.Reference<EntityType<?>> typeHolder() { return kind.builtInRegistryHolder(); }
    }
    static Object unsafe; static Method allocate;
    @SuppressWarnings("unchecked") static <T> T make(Class<T> type) throws Exception { return (T) allocate.invoke(unsafe, type); }

    static final class Resolver extends ItemModelResolver {
        Map<Identifier, ItemModel> models; Map<Identifier, ClientItem.Properties> properties; ItemModel missing;
        Resolver() { super(null); }
        @Override public void appendItemLayers(ItemStackRenderState output, ItemStack item, ItemDisplayContext context, Level level, ItemOwner owner, int seed) {
            Identifier id = item.get(DataComponents.ITEM_MODEL);
            if (id != null) {
                output.setOversizedInGui(properties.getOrDefault(id, ClientItem.Properties.DEFAULT).oversizedInGui());
                models.getOrDefault(id, missing).update(output, item, this, context, level instanceof ClientLevel c ? c : null, owner, seed);
            }
        }
        // ModelManager.getItemProperties, from the same baked properties.
        @Override public boolean shouldPlaySwapAnimation(ItemStack stack) {
            Identifier id = stack.get(DataComponents.ITEM_MODEL);
            return id == null || properties.getOrDefault(id, ClientItem.Properties.DEFAULT).handAnimationOnSwap();
        }
        @Override public float swapAnimationScale(ItemStack stack) {
            Identifier id = stack.get(DataComponents.ITEM_MODEL);
            return id == null ? 1.0F : properties.getOrDefault(id, ClientItem.Properties.DEFAULT).swapAnimationScale();
        }
    }

    static Object field(Object target, Class<?> type, String name) throws Exception {
        var f = type.getDeclaredField(name); f.setAccessible(true); return f.get(target);
    }
    static Object quad(BakedQuad q) {
        var positions = new long[4][3]; var uvs = new long[4][2];
        for (int i = 0; i < 4; i++) {
            var p = q.position(i); positions[i] = new long[]{bits(p.x()), bits(p.y()), bits(p.z())};
            long packed = q.packedUV(i); uvs[i] = new long[]{packed >>> 32, packed & 0xffffffffL};
        }
        var info = q.materialInfo();
        return map("positions", positions, "uv", uvs, "facing", q.direction().getName(), "tint", info.tintIndex(),
            "sprite", info.sprite().contents().name().toString(),
            "shade", info.shadeDirectionOverride() == null ? null : info.shadeDirectionOverride().getName(), "light", info.lightEmission());
    }
    static Object matrix(Matrix4f m) {
        var rows = new long[16];
        for (int r = 0; r < 4; r++) for (int c = 0; c < 4; c++) rows[r * 4 + c] = bits(m.getRowColumn(r, c));
        return rows;
    }
    static Object layer(ItemStackRenderState state, Object layer, ItemDisplayContext context) throws Exception {
        var type = layer.getClass();
        var out = new TreeMap<String, Object>();
        var special = (SpecialModelRenderer<?>) field(layer, type, "specialRenderer");
        var tints = (it.unimi.dsi.fastutil.ints.IntList) field(layer, type, "tintLayers");
        out.put("tints", tints == null ? new int[0] : tints.toIntArray());
        out.put("foil", ((ItemStackRenderState.FoilType) field(layer, type, "foilType")).name().toLowerCase(Locale.ROOT));
        var pose = new com.mojang.blaze3d.vertex.PoseStack.Pose();
        var apply = type.getDeclaredMethod("applyTransform", com.mojang.blaze3d.vertex.PoseStack.Pose.class);
        apply.setAccessible(true); apply.invoke(layer, pose);
        out.put("matrix", matrix(pose.pose()));
        out.put("uses_block_light", field(layer, type, "usesBlockLight"));
        var particle = (net.minecraft.client.resources.model.sprite.Material.Baked) field(layer, type, "particleMaterial");
        out.put("particle", particle == null ? null : particle.sprite().contents().name().toString());
        var transform = (net.minecraft.client.resources.model.cuboid.ItemTransform) field(layer, type, "itemTransform");
        out.put("has_transform", transform != net.minecraft.client.resources.model.cuboid.ItemTransform.NO_TRANSFORM);
        if (special instanceof TaggedRenderer tagged) {
            out.put("kind", "special");
            out.put("special", tagged.config);
            var argument = field(layer, type, "argumentForSpecialRendering");
            out.put("argument", argument instanceof Argument a ? a.json() : JsonNull.INSTANCE);
        } else {
            var quads = (net.minecraft.client.resources.model.geometry.ItemQuads) field(layer, type, "quads");
            out.put("quads", quads.all().stream().map(VanillaItemModelOracle::quad).toList());
            out.put("kind", "model");
        }
        return out;
    }

    static Map<String, JsonObject> prototypes = new HashMap<>();

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        // Item tags (compasses get the special foil) come from the vanilla data pack.
        var repo = net.minecraft.server.packs.repository.ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(List.of("vanilla"));
        try (var data = new net.minecraft.server.packs.resources.MultiPackResourceManager(net.minecraft.server.packs.PackType.SERVER_DATA, repo.openAllSelected())) {
            var layers = net.minecraft.server.RegistryLayer.createRegistryAccess();
            net.minecraft.tags.TagLoader.loadTagsForExistingRegistries(data, layers.getLayer(net.minecraft.server.RegistryLayer.STATIC))
                .forEach(net.minecraft.core.Registry.PendingTags::apply);
        }
        ItemModels.bootstrap();
        SpecialModelRenderers.bootstrap();
        net.minecraft.client.color.item.ItemTintSources.bootstrap();
        net.minecraft.client.renderer.item.properties.select.SelectItemModelProperties.bootstrap();
        net.minecraft.client.renderer.item.properties.conditional.ConditionalItemModelProperties.bootstrap();
        net.minecraft.client.renderer.item.properties.numeric.RangeSelectItemModelProperties.bootstrap();
        net.minecraft.client.renderer.texture.atlas.SpriteSources.bootstrap();
        var u = Class.forName("sun.misc.Unsafe");
        var uf = u.getDeclaredField("theUnsafe"); uf.setAccessible(true); unsafe = uf.get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        root = Path.of(args[0]);
        registries = VanillaRegistries.createWorldLookup();
        ops = RegistryOps.create(JsonOps.INSTANCE, registries);
        // Default components come from the prepared export of the vanilla reports.
        var defaults = JsonParser.parseString(Files.readString(Path.of(args[3]))).getAsJsonObject();
        for (var item : BuiltInRegistries.ITEM) {
            var json = defaults.get(BuiltInRegistries.ITEM.getKey(item).toString());
            var map = json == null ? net.minecraft.core.component.DataComponentMap.EMPTY
                : net.minecraft.core.component.DataComponentMap.CODEC.parse(ops, json).getOrThrow();
            item.builtInRegistryHolder().bindComponents(map);
        }

        // Grass colour map for the grass tint.
        try (var in = Files.newInputStream(root.resolve("assets/minecraft/textures/colormap/grass.png")); var image = NativeImage.read(in)) {
            GrassColor.init(image.getPixels());
        }

        // Every model file.
        var models = new HashMap<Identifier, UnbakedModel>();
        try (Stream<Path> files = Files.walk(root.resolve("assets/minecraft/models"))) {
            for (var file : (Iterable<Path>) files.filter(p -> p.toString().endsWith(".json"))::iterator) {
                var relative = root.resolve("assets/minecraft/models").relativize(file).toString().replace('\\', '/');
                try (var reader = Files.newBufferedReader(file)) {
                    models.put(Identifier.withDefaultNamespace(relative.substring(0, relative.length() - 5)), CuboidModel.fromStream(reader));
                } catch (RuntimeException ignored) {}
            }
        }
        // Every item definition.
        var output = Files.newBufferedWriter(Path.of(args[2]));
        output.write(JSON.toJson(map("row", "components", "names",
            BuiltInRegistries.DATA_COMPONENT_TYPE.keySet().stream().map(Identifier::toString).sorted().toList()))); output.newLine();
        var items = new TreeMap<Identifier, ClientItem>();
        try (Stream<Path> files = Files.list(root.resolve("assets/minecraft/items"))) {
            for (var file : (Iterable<Path>) files.sorted()::iterator) {
                var name = file.getFileName().toString();
                var id = Identifier.withDefaultNamespace(name.substring(0, name.length() - 5));
                String text = Files.readString(file);
                boolean ok = decode(id, text, items);
                output.write(JSON.toJson(map("row", "decode", "name", name, "ok", ok))); output.newLine();
            }
        }
        // Synthetic definitions from the harness.
        try (var lines = Files.lines(Path.of(args[1] + ".definitions"))) {
            for (var line : (Iterable<String>) lines::iterator) {
                var row = JsonParser.parseString(line).getAsJsonObject();
                var id = Identifier.parse(row.get("name").getAsString());
                boolean ok = decode(id, row.get("json").getAsString(), items);
                output.write(JSON.toJson(map("row", "decode", "name", row.get("name").getAsString(), "ok", ok))); output.newLine();
            }
        }

        var missingModel = MissingCuboidModel.missingModel();
        var discovery = new ModelDiscovery(models, missingModel);
        discovery.addSpecialModel(ItemModelGenerator.GENERATED_ITEM_MODEL_ID, new ItemModelGenerator());
        items.values().forEach(info -> discovery.addRoot(info.model()));
        var resolved = discovery.resolve();
        var missingSprite = new Sprite(BLOCKS, MissingTextureAtlasSprite.create());
        var blockAtlas = new SpriteLoader.Preparations(16, 16, 0, missingSprite, new Regions(false), CompletableFuture.completedFuture(null));
        var itemAtlas = new SpriteLoader.Preparations(16, 16, 0, missingSprite, new Regions(true), CompletableFuture.completedFuture(null));
        var materials = new net.minecraft.client.resources.model.sprite.MaterialBaker(blockAtlas, itemAtlas);
        var dummy = new Sprite(ITEMS, new SpriteContents(Identifier.parse("minecraft:dummy"), new net.minecraft.client.resources.metadata.animation.FrameSize(16, 16), new NativeImage(16, 16, true)));
        SpriteGetter sprites = id -> dummy;
        var bakery = new ModelBakery(EntityModelSet.vanilla(), sprites, null, Map.of(), tagged(items), resolved, discovery.missingModel());
        var baked = bakery.bakeModels(materials, Runnable::run).join();
        var resolver = new Resolver();
        resolver.models = baked.itemStackModels(); resolver.properties = baked.itemProperties(); resolver.missing = baked.missingModels().item();
        for (var id : items.keySet()) if (!resolver.models.containsKey(id)) {
            output.write(JSON.toJson(map("row", "unbaked", "name", id.toString()))); output.newLine();
        }
        var state = new ItemStackRenderState();

        try (var lines = Files.lines(Path.of(args[1]))) {
            for (var line : (Iterable<String>) lines::iterator) {
                var row = JsonParser.parseString(line).getAsJsonObject();
                var result = new TreeMap<String, Object>();
                result.put("row", "case"); result.put("id", row.get("id").getAsString());
                try {
                    var stackJson = new JsonObject();
                    stackJson.addProperty("id", row.get("item").getAsString());
                    stackJson.addProperty("count", row.get("count").getAsInt());
                    if (row.has("components")) stackJson.add("components", row.get("components"));
                    var stack = ItemStack.CODEC.parse(ops, stackJson).getOrThrow();
                    if (row.has("bundle_selected") && row.get("bundle_selected").getAsInt() >= 0) {
                        var contents = stack.get(DataComponents.BUNDLE_CONTENTS);
                        var mutable = contents.asMutable();
                        mutable.toggleSelectedItem(row.get("bundle_selected").getAsInt());
                        stack.set(DataComponents.BUNDLE_CONTENTS, mutable.toImmutable());
                    }
                    var context = ItemDisplayContext.CODEC.parse(JsonOps.INSTANCE, row.get("context")).getOrThrow();
                    frozenTime = row.has("local_time") ? row.get("local_time").getAsString() : "";
                    Actor actor = null;
                    if (row.has("owner") && !row.get("owner").isJsonNull()) {
                        var o = row.getAsJsonObject("owner");
                        actor = make(Actor.class);
                        actor.using = o.get("using").getAsBoolean();
                        actor.remaining = o.get("remaining").getAsInt();
                        actor.stack = stack;
                        actor.arm = o.get("main_arm").getAsString().equals("left") ? HumanoidArm.LEFT : HumanoidArm.RIGHT;
                        actor.kind = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(o.get("entity_type").getAsString()));
                        var derived = new TreeMap<String, Object>();
                        derived.put("use_duration", stack.getUseDuration(actor));
                        try { derived.put("charge_duration", CrossbowItem.getChargeDuration(stack, actor)); } catch (RuntimeException e) { derived.put("charge_duration", 0); }
                        result.put("derived", derived);
                    }
                    Level level = null;
                    if (row.has("dimension") && !row.get("dimension").isJsonNull()) {
                        var dim = net.minecraft.resources.ResourceKey.create(net.minecraft.core.registries.Registries.DIMENSION, Identifier.parse(row.get("dimension").getAsString()));
                        var clientLevel = make(ClientLevel.class);
                        var f = Level.class.getDeclaredField("dimension"); f.setAccessible(true);
                        var offset = (long) u.getMethod("objectFieldOffset", Field.class).invoke(unsafe, f);
                        u.getMethod("putObject", Object.class, long.class, Object.class).invoke(unsafe, clientLevel, offset, dim);
                        level = clientLevel;
                    }
                    resolver.updateForTopItem(state, stack, context, level, actor, row.has("seed") ? row.get("seed").getAsInt() : 0);
                    var layersField = ItemStackRenderState.class.getDeclaredField("layers"); layersField.setAccessible(true);
                    var countField = ItemStackRenderState.class.getDeclaredField("activeLayerCount"); countField.setAccessible(true);
                    var layers = (Object[]) layersField.get(state); int count = countField.getInt(state);
                    var list = new ArrayList<Object>();
                    for (int i = 0; i < count; i++) list.add(layer(state, layers[i], context));
                    result.put("layers", list);
                    result.put("oversized", state.isOversizedInGui());
                    result.put("swap", map("play", resolver.shouldPlaySwapAnimation(stack), "scale", bits(resolver.swapAnimationScale(stack))));
                } catch (Throwable e) {
                    Throwable cause = e; while (cause.getCause() != null) cause = cause.getCause();
                    result.put("error", cause.toString());
                }
                output.write(JSON.toJson(result)); output.newLine();
            }
        }
        output.close();
    }

    static Map<Identifier, ClientItem> tagged(Map<Identifier, ClientItem> items) {
        var out = new TreeMap<Identifier, ClientItem>();
        items.forEach((id, item) -> out.put(id, new ClientItem(tag(item.model()), item.properties(), item.registrySwapper())));
        return out;
    }
    static boolean decode(Identifier id, String text, Map<Identifier, ClientItem> items) {
        try {
            var parsed = ClientItem.CODEC.parse(ops, StrictJsonParser.parse(new StringReader(text))).result();
            parsed.ifPresent(item -> items.put(id, item));
            return parsed.isPresent();
        } catch (Exception e) { return false; }
    }
}
