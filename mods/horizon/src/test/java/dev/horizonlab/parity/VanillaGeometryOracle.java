package dev.horizonlab.parity;

import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.Entity;
import net.minecraft.util.Mth;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;

/** Original I/O adapter. Every geometry calculation invokes the pinned vanilla classes. */
public final class VanillaGeometryOracle {
    private static final Constructor<ArrayVoxelShape> ARRAY;
    private static final Method INDEX;
    private static final Method MOTION, STEP, COLLIDE, ALLOCATE;
    private static final Object ALLOCATOR;
    static {
        try {
            ARRAY = ArrayVoxelShape.class.getDeclaredConstructor(DiscreteVoxelShape.class, double[].class, double[].class, double[].class);
            ARRAY.setAccessible(true);
            INDEX = VoxelShape.class.getDeclaredMethod("findIndex", Direction.Axis.class, double.class);
            INDEX.setAccessible(true);
            MOTION = Entity.class.getDeclaredMethod("collideWithShapes", Vec3.class, AABB.class, List.class); MOTION.setAccessible(true);
            STEP = Entity.class.getDeclaredMethod("collectCandidateStepUpHeights", AABB.class, List.class, float.class, float.class); STEP.setAccessible(true);
            COLLIDE = Entity.class.getDeclaredMethod("collide", Vec3.class); COLLIDE.setAccessible(true);
            Class<?> allocator = Class.forName("sun.misc.Unsafe");
            Field field = allocator.getDeclaredField("theUnsafe"); field.setAccessible(true);
            ALLOCATOR = field.get(null); ALLOCATE = allocator.getMethod("allocateInstance", Class.class);
        } catch (ReflectiveOperationException error) { throw new ExceptionInInitializerError(error); }
    }
    private static final class Input {
        final String[] words; int cursor = 1;
        Input(String line) { words = line.split(" "); }
        String word() { return words[cursor++]; }
        int integer() { return Integer.parseInt(word()); }
        double number() { return Double.longBitsToDouble(Long.parseUnsignedLong(word(), 16)); }
        Vec3 vector() { return new Vec3(number(), number(), number()); }
        AABB box() { return new AABB(vector(), vector()); }
        BlockPos pos() { return new BlockPos(integer(), integer(), integer()); }
    }
    private final PrintWriter out;
    private final List<VoxelShape> shapes = new ArrayList<>();
    private VanillaGeometryOracle(PrintWriter out) { this.out = out; }
    private void values(String label, double... values) { out.print(label); for (double v : values) out.printf(" %016x", Double.doubleToLongBits(v)); out.println(); }
    private void vector(String label, Vec3 v) { values(label, v.x, v.y, v.z); }
    private void box(String label, AABB b) { values(label, b.minX, b.minY, b.minZ, b.maxX, b.maxY, b.maxZ); }
    private void point(String label, Optional<Vec3> p) { if (p.isPresent()) vector(label, p.get()); else out.println(label + " none"); }
    private void hit(BlockHitResult h) { if (h == null) out.println("hit none"); else { BlockPos p = h.getBlockPos(); Vec3 v = h.getLocation(); out.printf("hit %d %d %d %d %d %016x %016x %016x%n", h.getDirection().ordinal(), h.isInside() ? 1 : 0, p.getX(), p.getY(), p.getZ(), Double.doubleToLongBits(v.x), Double.doubleToLongBits(v.y), Double.doubleToLongBits(v.z)); } }
    private static int flag(boolean v) { return v ? 1 : 0; }
    // These fixtures only provide caller-controlled shape lists and entity
    // inputs. The private vanilla Entity.collide method performs every movement
    // and step decision. Allocation avoids starting a world/server or invoking
    // entity lifecycle logic unrelated to this geometry comparison.
    private static final class StepBorder extends net.minecraft.world.level.border.WorldBorder {
        VoxelShape shape;
        @Override public boolean isInsideCloseToBorder(Entity entity, AABB box) { return shape != null; }
        @Override public VoxelShape getCollisionShape() { return shape; }
    }
    private static final class StepLevel extends net.minecraft.server.level.ServerLevel {
        VanillaGeometryOracle oracle;
        StepBorder border;
        List<VoxelShape> entities, initial, stepping;
        int queries;
        private StepLevel() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        @Override public net.minecraft.world.level.border.WorldBorder getWorldBorder() { return border; }
        @Override public List<VoxelShape> getEntityCollisions(Entity entity, AABB query) { oracle.box("entity-query", query); return entities; }
        @Override public Iterable<VoxelShape> getBlockCollisions(Entity entity, AABB query) { oracle.box("block-query", query); return queries++ == 0 ? initial : stepping; }
    }
    private static final class StepEntity extends net.minecraft.world.entity.decoration.ArmorStand {
        float step;
        boolean grounded;
        private StepEntity() { super(null, null); }
        @Override public float maxUpStep() { return step; }
        @Override public boolean onGround() { return grounded; }
        void configure(StepLevel level, AABB box, float step, boolean grounded) { setLevel(level); setBoundingBox(box); this.step = step; this.grounded = grounded; }
    }
    private List<VoxelShape> selected(Input in) {
        int count = in.integer(); List<VoxelShape> selected = new ArrayList<>(count);
        for (int i = 0; i < count; i++) selected.add(shapes.get(in.integer()));
        return selected;
    }
    private void positions(String label, Iterable<BlockPos> positions, int limit) {
        int count = 0; Iterator<BlockPos> iterator = positions.iterator();
        while (count < limit && iterator.hasNext()) { BlockPos pos = iterator.next(); out.printf("%s %d %d %d%n", label, pos.getX(), pos.getY(), pos.getZ()); count++; }
        out.printf("%s-count %d%n", label, count);
    }
    private void emitShape(VoxelShape shape) {
        values("axis-extrema",shape.min(Direction.Axis.X),shape.min(Direction.Axis.Y),shape.min(Direction.Axis.Z),shape.max(Direction.Axis.X),shape.max(Direction.Axis.Y),shape.max(Direction.Axis.Z));
        List<AABB> boxes=shape.toAabbs();out.printf("shape %d %d%n",shapes.size(),boxes.size());int cubeAxes=0;
        for(Direction.Axis axis:Direction.Axis.values())if(shape.getCoords(axis) instanceof CubePointRange)cubeAxes|=1<<axis.ordinal();
        out.printf("cube %d %d%n",flag(shape instanceof CubeVoxelShape),cubeAxes);
        for(Direction.Axis axis:Direction.Axis.values())values(axis.getName()+"-coords",shape.getCoords(axis).toDoubleArray());
        if(shape.isEmpty())out.println("shape-bounds none");else box("shape-bounds",shape.bounds());for(AABB box:boxes)box("part",box);shapes.add(shape);
    }
    private void command(String line, int lineNumber) throws ReflectiveOperationException {
        Input in = new Input(line); String operation = in.words[0]; out.printf("command %d %s%n", lineNumber, operation);
        switch (operation) {
            case "floor-height" -> {
                VoxelShape shape = shapes.get(in.integer()), below = shapes.get(in.integer()); int[] calls = {0};
                values("floor-height", net.minecraft.world.level.EmptyBlockGetter.INSTANCE.getBlockFloorHeight(shape, () -> { calls[0]++; return below; }));
                out.printf("below-calls %d%n", calls[0]);
            }
            case "block-iterate" -> {
                BlockPos a = in.pos(), b = in.pos(); Vec3 movement = in.vector(); int limit = in.integer();
                positions("closed", BlockPos.betweenClosed(a, b), limit);
                positions("direction", BlockPos.betweenCornersInDirection(a, b, movement), limit);
            }
            case "box-iterate" -> {
                AABB box = in.box(); Vec3 movement = in.vector(); int limit = in.integer();
                positions("closed", BlockPos.betweenClosed(box), limit);
                positions("direction", BlockPos.betweenCornersInDirection(box, movement), limit);
            }
            case "sweep" -> {
                Vec3 start = in.vector(), end = in.vector(); AABB box = in.box(); int stop = in.integer(); int[] count = {0};
                boolean completed = BlockGetter.forEachBlockIntersectedBetween(start, end, box, (pos, step) -> { count[0]++; out.printf("block-visit %d %d %d %d%n", pos.getX(), pos.getY(), pos.getZ(), step); return count[0] != stop; });
                out.printf("sweep %d %d%n", flag(completed), count[0]);
            }
            case "step-motion" -> {
                AABB box = in.box(); Vec3 movement = in.vector(); float maxStep = (float)in.number(); boolean grounded = in.integer() != 0;
                StepLevel level = (StepLevel)ALLOCATE.invoke(ALLOCATOR, StepLevel.class);
                level.oracle = this; level.entities = selected(in); level.initial = selected(in); level.stepping = selected(in);
                int border = in.integer(); level.border = new StepBorder(); level.border.shape = border < 0 ? null : shapes.get(border);
                StepEntity entity = (StepEntity)ALLOCATE.invoke(ALLOCATOR, StepEntity.class); entity.configure(level, box, maxStep, grounded);
                vector("step-motion", (Vec3)COLLIDE.invoke(entity, movement));
                out.printf("block-queries %d%n", level.queries);
            }
            case "make-box" -> emitShape(Shapes.create(in.box()));
            case "optimize" -> emitShape(shapes.get(in.integer()).optimize());
            case "join", "join-optimized" -> {
                VoxelShape a=shapes.get(in.integer()),b=shapes.get(in.integer());int bits=in.integer();BooleanOp op=(first,second)->(bits&(1<<((first?2:0)+(second?1:0))))!=0;
                if((bits&1)!=0) {
                    try {Shapes.joinUnoptimized(a,b,op);throw new AssertionError("accepted unbounded Boolean");}catch(IllegalArgumentException expected){out.println("join invalid");}
                }else{
                    VoxelShape shape=operation.equals("join")?Shapes.joinUnoptimized(a,b,op):Shapes.join(a,b,op);out.printf("overlap %d%n",flag(Shapes.joinIsNotEmpty(a,b,op)));emitShape(shape);
                }
            }
            case "traverse" -> {
                Vec3 start=in.vector(),end=in.vector();int stop=in.integer();int[] count={0};
                BlockPos result=BlockGetter.traverseBlocks(start,end,stop,(context,pos)->{count[0]++;out.printf("visit %d %d %d%n",pos.getX(),pos.getY(),pos.getZ());if(count[0]==context)return pos;return null;},context->null);
                if(result==null)out.printf("traverse none %d%n",count[0]);else out.printf("traverse %d %d %d %d%n",result.getX(),result.getY(),result.getZ(),count[0]);
            }
            case "trig-table" -> {
                Field field=Mth.class.getDeclaredField("SIN");field.setAccessible(true);float[] table=(float[])field.get(null);
                for(int i=0;i<table.length;i++)out.printf("sine %d %08x%n",i,Float.floatToIntBits(table[i]));
            }
            case "rotation" -> {
                Vec3 v=in.vector();double angle=in.number();float pitch=(float)in.number(),yaw=(float)in.number();int axes=in.integer();Direction.Axis axis=Direction.Axis.values()[in.integer()];Direction direction=Direction.values()[in.integer()];double amount=in.number();
                values("trig",Mth.sin(angle),Mth.cos(angle));vector("rotate-x",v.xRot((float)angle));vector("rotate-y",v.yRot((float)angle));vector("rotate-z",v.zRot((float)angle));vector("clockwise",v.rotateClockwise90());vector("look",Vec3.directionFromRotation(pitch,yaw));
                Vec2 rotation=v.rotation();values("rotation",rotation.x,rotation.y);
                EnumSet<Direction.Axis> selected=EnumSet.noneOf(Direction.Axis.class);for(Direction.Axis a:Direction.Axis.values())if((axes&(1<<a.ordinal()))!=0)selected.add(a);
                vector("align",v.align(selected));vector("with",v.with(axis,amount));vector("relative",v.relative(direction,amount));
            }
            case "vec" -> {
                Vec3 a = in.vector(), b = in.vector(); double t = in.number();
                vector("add", a.add(b)); vector("subtract", a.subtract(b)); vector("vector-to", a.vectorTo(b)); vector("multiply", a.multiply(b)); vector("scale", a.scale(t)); vector("normalize", a.normalize()); vector("cross", a.cross(b)); vector("lerp", a.lerp(b, t)); vector("projected", a.projectedOn(b)); vector("reverse", a.reverse()); vector("horizontal", a.horizontal());
                values("scalar", a.dot(b), a.lengthSqr(), a.length(), a.horizontalDistanceSqr(), a.horizontalDistance(), a.distanceToSqr(b), a.distanceTo(b));
                out.printf("flags %d %d %d %d%n", flag(a.equals(b)), flag(a.isFinite()), flag(a.closerThan(b, t)), Direction.getApproximateNearest(a).ordinal());
                var order = Direction.axisStepOrder(a); out.printf("order %d %d %d%n", order.get(0).ordinal(), order.get(1).ordinal(), order.get(2).ordinal());
            }
            case "box" -> {
                AABB a = in.box(), b = in.box(); Vec3 start = in.vector(), end = in.vector(), amount = in.vector(); Direction.Axis axis = Direction.Axis.values()[in.integer()]; double value = in.number();
                box("bounds", a); box("move", a.move(amount)); box("inflate", a.inflate(amount.x, amount.y, amount.z)); box("deflate", a.deflate(amount.x, amount.y, amount.z)); box("contract", a.contract(amount.x, amount.y, amount.z)); box("expand", a.expandTowards(amount)); box("intersect", a.intersect(b)); box("union", a.minmax(b)); box("next", a.nextDeflated());
                box("set-min", switch(axis) { case X -> a.setMinX(value); case Y -> a.setMinY(value); case Z -> a.setMinZ(value); });
                box("set-max", switch(axis) { case X -> a.setMaxX(value); case Y -> a.setMaxY(value); case Z -> a.setMaxZ(value); });
                box("of-size", AABB.ofSize(start, amount.x, amount.y, amount.z)); box("unit", AABB.unitCubeFromLowerCorner(start));
                vector("center", a.getCenter()); vector("bottom", a.getBottomCenter()); vector("size", new Vec3(a.getXsize(), a.getYsize(), a.getZsize()));
                values("distance", a.getSize(), a.distanceToSqr(start), a.distanceToSqr(b));
                out.printf("flags %d %d %d %d%n", flag(a.intersects(b)), flag(a.contains(start)), flag(a.hasNaN()), flag(a.collidedAlongVector(amount, List.of(b))));
                point("clip", a.clip(start, end)); hit(AABB.clip(List.of(a, b), start, end, BlockPos.ZERO));
            }
            case "block-box" -> { BlockPos a = in.pos(), b = in.pos(); box("block", new AABB(a)); box("encapsulate", AABB.encapsulatingFullBlocks(a, b)); }
            case "grid" -> {
                boolean cube = in.integer() != 0; int nx = in.integer(), ny = in.integer(), nz = in.integer();
                double[] xs = new double[nx + 1], ys = new double[ny + 1], zs = new double[nz + 1];
                for (double[] coords : new double[][] {xs, ys, zs}) for (int i = 0; i < coords.length; i++) coords[i] = in.number();
                String occupied = in.word(); BitSetDiscreteVoxelShape discrete = new BitSetDiscreteVoxelShape(nx, ny, nz);
                for (int x = 0; x < nx; x++) for (int y = 0; y < ny; y++) for (int z = 0; z < nz; z++) if (occupied.charAt((x * ny + y) * nz + z) == '1') discrete.fill(x, y, z);
                VoxelShape shape = cube ? new CubeVoxelShape(discrete) : ARRAY.newInstance(discrete, xs, ys, zs);
                List<AABB> boxes = shape.toAabbs(); out.printf("shape %d %d%n", shapes.size(), boxes.size());
                values("axis-extrema",shape.min(Direction.Axis.X),shape.min(Direction.Axis.Y),shape.min(Direction.Axis.Z),shape.max(Direction.Axis.X),shape.max(Direction.Axis.Y),shape.max(Direction.Axis.Z));
                if (shape.isEmpty()) out.println("shape-bounds none"); else box("shape-bounds", shape.bounds());
                for (AABB b : boxes) box("part", b); shapes.add(shape);
            }
            case "query" -> {
                VoxelShape shape = shapes.get(in.integer()); AABB b = in.box(); Vec3 start = in.vector(), end = in.vector(), p = in.vector(); BlockPos block = in.pos(); Direction.Axis axis = Direction.Axis.values()[in.integer()]; double amount = in.number();
                values("column", shape.min(axis, p.x, p.y), shape.max(axis, p.x, p.y));
                values("collide", shape.collide(axis, b, amount)); hit(shape.clip(start, end, block)); point("closest", shape.closestPointTo(p));
                out.printf("index %d %d %d%n", INDEX.invoke(shape, Direction.Axis.X, p.x), INDEX.invoke(shape, Direction.Axis.Y, p.y), INDEX.invoke(shape, Direction.Axis.Z, p.z));
            }
            case "many" -> {
                AABB b = in.box(); Direction.Axis axis = Direction.Axis.values()[in.integer()]; double amount = in.number(); int count = in.integer(); List<VoxelShape> selected = new ArrayList<>();
                for (int i = 0; i < count; i++) selected.add(shapes.get(in.integer())); values("many", Shapes.collide(axis, b, selected, amount));
            }
            case "motion" -> {
                AABB b = in.box(); Vec3 movement = in.vector(); float maxStep = (float)in.number(), currentY = (float)in.number(); int count = in.integer(); List<VoxelShape> selected = new ArrayList<>();
                for (int i = 0; i < count; i++) selected.add(shapes.get(in.integer()));
                vector("motion", (Vec3)MOTION.invoke(null,movement,b,selected));
                float[] candidates = (float[])STEP.invoke(null,b,selected,maxStep,currentY);
                out.printf("steps %d",candidates.length); for(float height:candidates)out.printf(" %08x",Float.floatToIntBits(height)); out.println();
            }
            default -> throw new IllegalArgumentException(operation);
        }
        if (in.cursor != in.words.length) throw new AssertionError("trailing input");
    }
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion(); Bootstrap.bootStrap();
        try (BufferedReader input = Files.newBufferedReader(Path.of(args[0])); PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))) {
            VanillaGeometryOracle oracle = new VanillaGeometryOracle(out); int lineNumber = 0;
            for (String line; (line = input.readLine()) != null;) oracle.command(line, ++lineNumber);
        }
    }
}
