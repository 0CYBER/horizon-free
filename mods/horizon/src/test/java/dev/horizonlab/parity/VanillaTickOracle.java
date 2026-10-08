package dev.horizonlab.parity;

import java.io.*;
import java.lang.reflect.Field;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Vec3i;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.ticks.*;

/** Original trace adapter; queues, ordering, mutations and save packing are vanilla. */
public final class VanillaTickOracle {
    private static final Object[] TYPES = new Object[32];
    static { for (int i = 0; i < TYPES.length; i++) TYPES[i] = new Object(); }
    private static final Field ALREADY_RUN;
    static {
        try { ALREADY_RUN = LevelTicks.class.getDeclaredField("alreadyRunThisTick"); ALREADY_RUN.setAccessible(true); }
        catch (ReflectiveOperationException error) { throw new ExceptionInInitializerError(error); }
    }
    private final Set<Long> blocked = new HashSet<>();
    private final Map<Long, LevelChunkTicks<Object>> chunks = new HashMap<>();
    private final Map<Long, LevelChunkTicks<Object>> detached = new HashMap<>();
    private LevelTicks<Object> level = new LevelTicks<>(key -> !blocked.contains(key));
    private final PrintWriter out;
    private VanillaTickOracle(PrintWriter out) { this.out = out; }
    private static int type(Object type) { for (int i = 0; i < TYPES.length; i++) if (TYPES[i] == type) return i; throw new AssertionError(); }
    private static final class Input {
        final String[] words; int cursor = 1;
        Input(String line) { words = line.split(" "); }
        int integer() { return Integer.parseInt(words[cursor++]); }
        long number() { return Long.parseLong(words[cursor++]); }
        BlockPos pos() { return new BlockPos(integer(), integer(), integer()); }
        ChunkPos chunk() { return new ChunkPos(integer(), integer()); }
        BoundingBox area() { BlockPos a = pos(), b = pos(); return new BoundingBox(Math.min(a.getX(), b.getX()), Math.min(a.getY(), b.getY()), Math.min(a.getZ(), b.getZ()), Math.max(a.getX(), b.getX()), Math.max(a.getY(), b.getY()), Math.max(a.getZ(), b.getZ())); }
    }
    private int schedule(ScheduledTick<Object> tick) {
        if (!chunks.containsKey(ChunkPos.pack(tick.pos()))) { level.schedule(tick); return 2; }
        boolean duplicate = level.hasScheduledTick(tick.pos(), tick.type());
        level.schedule(tick);
        return duplicate ? 0 : 1;
    }
    private void add(ChunkPos pos, LevelChunkTicks<Object> chunk) { chunks.put(pos.pack(), chunk); level.addContainer(pos, chunk); }
    private LevelChunkTicks<Object> remove(ChunkPos pos) { level.removeContainer(pos); return chunks.remove(pos.pack()); }
    private void scheduled(ScheduledTick<Object> tick) { out.printf("queued %d %d %d %d %d %d %d%n", type(tick.type()), tick.pos().getX(), tick.pos().getY(), tick.pos().getZ(), tick.triggerTick(), tick.priority().getValue(), tick.subTickOrder()); }
    @SuppressWarnings("unchecked")
    private ScheduledTick<Object> running() {
        try { List<ScheduledTick<Object>> ticks = (List<ScheduledTick<Object>>) ALREADY_RUN.get(level); return ticks.getLast(); }
        catch (ReflectiveOperationException error) { throw new AssertionError(error); }
    }
    private void command(String line, int lineNumber) {
        Input in = new Input(line); String operation = in.words[0];
        out.printf("command %d %s%n", lineNumber, operation);
        switch (operation) {
            case "reset" -> { blocked.clear(); chunks.clear(); detached.clear(); level = new LevelTicks<>(key -> !blocked.contains(key)); }
            case "add" -> { ChunkPos pos = in.chunk(); if (!chunks.containsKey(pos.pack())) add(pos, new LevelChunkTicks<>()); }
            case "load" -> {
                ChunkPos pos = in.chunk(); int count = in.integer(); List<SavedTick<Object>> ticks = new ArrayList<>();
                for (int i = 0; i < count; i++) ticks.add(new SavedTick<>(TYPES[in.integer()], in.pos(), in.integer(), TickPriority.byValue(in.integer())));
                remove(pos); add(pos, new LevelChunkTicks<>(ticks));
            }
            case "unpack" -> { ChunkPos pos = in.chunk(); long time = in.number(); LevelChunkTicks<Object> chunk = chunks.get(pos.pack()); if (chunk != null) chunk.unpack(time); }
            case "remove" -> { ChunkPos pos = in.chunk(); LevelChunkTicks<Object> chunk = remove(pos); if (chunk != null) detached.put(pos.pack(), chunk); }
            case "restore" -> { ChunkPos pos = in.chunk(); if (!chunks.containsKey(pos.pack())) { LevelChunkTicks<Object> chunk = detached.remove(pos.pack()); if (chunk != null) add(pos, chunk); } }
            case "enable" -> { ChunkPos pos = in.chunk(); if (in.integer() == 0) blocked.add(pos.pack()); else blocked.remove(pos.pack()); }
            case "schedule" -> { ScheduledTick<Object> tick = new ScheduledTick<>(TYPES[in.integer()], in.pos(), in.number(), TickPriority.byValue(in.integer()), in.number()); out.printf("schedule %d%n", schedule(tick)); }
            case "query" -> { Object kind = TYPES[in.integer()]; BlockPos pos = in.pos(); out.printf("query %d %d%n", level.hasScheduledTick(pos, kind) ? 1 : 0, level.willTickThisTick(pos, kind) ? 1 : 0); }
            case "chunk" -> { LevelChunkTicks<Object> chunk = chunks.get(in.chunk().pack()); if (chunk == null) out.println("chunk absent"); else { out.printf("chunk %d%n", chunk.count()); chunk.getAll().forEach(this::scheduled); } }
            case "save" -> {
                LevelChunkTicks<Object> chunk = chunks.get(in.chunk().pack()); long time = in.number();
                if (chunk == null) out.println("saved absent"); else {
                    List<SavedTick<Object>> ticks = chunk.pack(time); out.printf("saved %d%n", ticks.size());
                    for (SavedTick<Object> tick : ticks) out.printf("saved-value %d %d %d %d %d %d%n", type(tick.type()), tick.pos().getX(), tick.pos().getY(), tick.pos().getZ(), tick.delay(), tick.priority().getValue());
                }
            }
            case "reload" -> { ChunkPos pos = in.chunk(); long saveTime = in.number(), loadTime = in.number(); LevelChunkTicks<Object> old = remove(pos); if (old != null) { LevelChunkTicks<Object> chunk = new LevelChunkTicks<>(old.pack(saveTime)); chunk.unpack(loadTime); add(pos, chunk); } }
            case "clear" -> level.clearArea(in.area());
            case "copy" -> { BoundingBox area = in.area(); BlockPos offset = in.pos(); level.copyArea(area, new Vec3i(offset.getX(), offset.getY(), offset.getZ())); }
            case "tick" -> {
                long time = in.number(); int maximum = in.integer(), action = in.integer(); Object probeKind = TYPES[in.integer()]; BlockPos probe = in.pos(); long[] order = {in.number()};
                level.tick(time, maximum, (pos, kind) -> {
                    ScheduledTick<Object> tick = running(); scheduled(tick);
                    out.printf("running %d %d %d%n", level.willTickThisTick(pos, kind) ? 1 : 0, level.hasScheduledTick(pos, kind) ? 1 : 0, level.willTickThisTick(probe, probeKind) ? 1 : 0);
                    switch (action) {
                        case 1 -> out.printf("callback-schedule %d%n", schedule(new ScheduledTick<>(kind, pos, time, tick.priority(), order[0]++)));
                        case 2 -> { level.clearArea(new BoundingBox(pos.getX(), pos.getY(), pos.getZ(), pos.getX() + 1, pos.getY() + 1, pos.getZ() + 1)); out.printf("callback-clear %d%n", level.willTickThisTick(probe, probeKind) ? 1 : 0); }
                        case 3 -> level.copyArea(new BoundingBox(pos.getX() - 2, pos.getY() - 2, pos.getZ() - 2, pos.getX() + 2, pos.getY() + 2, pos.getZ() + 2), new Vec3i(16, 0, 0));
                        case 4 -> { ChunkPos chunkPos = new ChunkPos(pos.getX() >> 4, pos.getZ() >> 4); LevelChunkTicks<Object> chunk = remove(chunkPos); if (chunk != null) detached.put(chunkPos.pack(), chunk); }
                        case 5 -> blocked.add(ChunkPos.pack(pos));
                        case 0 -> {}
                        default -> throw new AssertionError("unknown callback action");
                    }
                });
            }
            default -> throw new AssertionError("unknown command: " + operation);
        }
        if (in.cursor != in.words.length) throw new AssertionError("extra arguments");
        out.printf("count %d%n", level.count());
    }
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        try (BufferedReader input = Files.newBufferedReader(Path.of(args[0])); PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))) {
            VanillaTickOracle oracle = new VanillaTickOracle(out); String line; int index = 0;
            while ((line = input.readLine()) != null) oracle.command(line, index++);
        }
    }
}
