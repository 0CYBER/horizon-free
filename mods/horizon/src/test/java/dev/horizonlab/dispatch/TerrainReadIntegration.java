package dev.horizonlab.dispatch;

import com.seibel.distanthorizons.core.util.ExceptionUtil;
import dev.horizonlab.mixin.HorizonMixinPlugin;
import it.unimi.dsi.fastutil.longs.Long2ObjectLinkedOpenHashMap;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.BitSet;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientLifecycleEvents;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.chunk.storage.IOWorker;
import net.minecraft.world.level.chunk.storage.RegionStorageInfo;

/** Applies the redirect to a real game IOWorker, using controlled cached read futures. */
public final class TerrainReadIntegration implements ClientModInitializer {
    private static final class Worker extends IOWorker {
        Worker(Path path) { super(new RegionStorageInfo("fixture", Level.OVERWORLD, "horizon-read-fixture"), path, false); }
    }
    private record Outcome(Boolean result, Throwable error, boolean interrupted) {}
    private record Call(Thread thread, CountDownLatch finished, AtomicReference<Outcome> outcome) {}

    @Override public void onInitializeClient() {
        ClientLifecycleEvents.CLIENT_STARTED.register(client -> {
            try { exercise(); }
            catch (Exception failure) { throw new AssertionError("Terrain read integration failed", failure); }
            if (Boolean.getBoolean("horizon.fixture.exit")) System.exit(0);
        });
    }

    @SuppressWarnings("unchecked")
    private static void cache(Worker worker, CompletableFuture<BitSet> future) throws Exception {
        var field = IOWorker.class.getDeclaredField("regionCacheForBlender");
        field.setAccessible(true);
        var map = (Long2ObjectLinkedOpenHashMap<CompletableFuture<BitSet>>) field.get(worker);
        synchronized (map) { map.clear(); map.put(ChunkPos.pack(0, 0), future); }
    }

    private static Call start(Worker worker, String name, boolean initiallyInterrupted) {
        var result = new AtomicReference<Outcome>();
        var finished = new CountDownLatch(1);
        Thread thread = new Thread(() -> {
            if (initiallyInterrupted) Thread.currentThread().interrupt();
            Boolean value = null; Throwable error = null;
            try { value = worker.isOldChunkAround(new ChunkPos(0, 0), 0); }
            catch (Throwable failure) { error = failure; }
            result.set(new Outcome(value, error, Thread.currentThread().isInterrupted()));
            finished.countDown();
        }, name);
        thread.setDaemon(true); // Fixture failures cannot strand the test JVM.
        thread.start();
        return new Call(thread, finished, result);
    }

    private static Outcome finish(Call call) throws Exception {
        if (!call.finished.await(4, TimeUnit.SECONDS)) throw new AssertionError("Worker did not finish");
        call.thread.join(1000);
        return call.outcome.get();
    }

    private static void awaitPending(Call call) throws Exception {
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(4);
        while (call.thread.getState() != Thread.State.WAITING) {
            if (!call.thread.isAlive() || System.nanoTime() > deadline) throw new AssertionError("Read never waited");
            Thread.sleep(1);
        }
    }

    private static void pending(Worker worker, String name, boolean cancellable) throws Exception {
        var future = new CompletableFuture<BitSet>();
        cache(worker, future);
        var call = start(worker, name, false);
        try {
            awaitPending(call);
            call.thread.interrupt();
            if (cancellable) {
                var outcome = finish(call);
                if (!(outcome.error instanceof CompletionException) || !ExceptionUtil.isShutdownException(outcome.error)
                    || !outcome.interrupted || future.isDone()) throw new AssertionError("Shutdown changed shared future or interrupt");
            } else {
                if (call.finished.await(100, TimeUnit.MILLISECONDS)) throw new AssertionError("Ordinary join became interruptible");
            }
            future.complete(new BitSet());
            var outcome = finish(call);
            if (!cancellable && (!Boolean.FALSE.equals(outcome.result) || outcome.error != null || !outcome.interrupted))
                throw new AssertionError("Original join result or interrupt changed");
            // A later caller can still consume the shared read after cancellation of only one waiter.
            if (worker.isOldChunkAround(new ChunkPos(0, 0), 0)) throw new AssertionError("Shared read was damaged");
        } finally { future.complete(new BitSet()); call.thread.join(4000); }
    }

    private static void exercise() throws Exception {
        boolean enabled = HorizonMixinPlugin.terrainReadWaitEnabled();
        Path path = Files.createTempDirectory("horizon-read-fixture-");
        try (var worker = new Worker(path)) {
            pending(worker, "DH-World Gen Thread[fixture]", enabled);
            pending(worker, "Server thread fixture", false);
            pending(worker, "DH-LOD Builder Thread[fixture]", false);
            var waiting = new CompletableFuture<BitSet>();
            cache(worker, waiting);
            var initialCall = start(worker, "DH-World Gen Thread[fixture]", true);
            try {
                if (enabled) {
                    var preInterrupted = finish(initialCall);
                    if (!(preInterrupted.error instanceof CompletionException) || !preInterrupted.interrupted
                        || waiting.isDone() || !ExceptionUtil.isShutdownException(preInterrupted.error))
                        throw new AssertionError("Pending read ignored an existing shutdown interrupt");
                } else {
                    awaitPending(initialCall);
                    if (initialCall.finished.await(100, TimeUnit.MILLISECONDS))
                        throw new AssertionError("Original pending join honored an existing interrupt");
                    waiting.complete(new BitSet());
                    var preInterrupted = finish(initialCall);
                    if (!Boolean.FALSE.equals(preInterrupted.result) || preInterrupted.error != null || !preInterrupted.interrupted)
                        throw new AssertionError("Original pre-interrupted join changed");
                }
            } finally { waiting.complete(new BitSet()); initialCall.thread.join(4000); }
            BitSet bits = new BitSet(); bits.set(0);
            cache(worker, CompletableFuture.completedFuture(bits));
            var complete = finish(start(worker, "DH-World Gen Thread[fixture]", true));
            if (!Boolean.TRUE.equals(complete.result) || complete.error != null || !complete.interrupted)
                throw new AssertionError("Completed read changed result or interrupt");
            var failed = new CompletableFuture<BitSet>();
            var original = new CompletionException(new IllegalStateException("fixture failure"));
            cache(worker, failed);
            var call = start(worker, "DH-World Gen Thread[fixture]", false);
            awaitPending(call); failed.completeExceptionally(original);
            if (finish(call).error != original) throw new AssertionError("Exceptional completion identity changed");
            var pendingCancelled = new CompletableFuture<BitSet>();
            cache(worker, pendingCancelled);
            var cancelCall = start(worker, "DH-World Gen Thread[fixture]", false);
            awaitPending(cancelCall); pendingCancelled.cancel(false);
            Throwable expectedCancellation = null;
            try { pendingCancelled.join(); } catch (java.util.concurrent.CancellationException expected) { expectedCancellation = expected; }
            var actualCancellation = finish(cancelCall).error;
            if (!(actualCancellation instanceof java.util.concurrent.CancellationException)
                || !java.util.Objects.equals(actualCancellation.getMessage(), expectedCancellation.getMessage())
                || actualCancellation.getCause() != expectedCancellation.getCause())
                throw new AssertionError("Pending cancellation changed join's message or cause");
            var cancelled = new CompletableFuture<BitSet>(); cancelled.cancel(false);
            cache(worker, cancelled);
            if (!(finish(start(worker, "DH-World Gen Thread[fixture]", false)).error instanceof java.util.concurrent.CancellationException))
                throw new AssertionError("Cancelled completion changed");
            System.out.println("HORIZON_TERRAIN_READ_RESULT {\"enabled\":" + enabled
                + ",\"applied_io_worker_checked\":true,\"ordinary_join_checked\":true,\"shared_future_preserved\":true"
                + ",\"exception_identity_checked\":true,\"completed_interrupt_checked\":true,\"cancelled_completion_checked\":true,\"preinterrupted_pending_checked\":true}");
        } finally { Files.deleteIfExists(path); }
    }
}
