package dev.horizonlab.budget;

import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CancellationException;
import java.util.concurrent.CompletionException;
import java.util.concurrent.ExecutionException;

/** Allow the pinned DH generation pool's shutdown interrupt to release a read-only wait. */
public final class TerrainReadWait {
    private TerrainReadWait() {}

    public static <T> T join(CompletableFuture<T> future) {
        // Ordinary game callers retain join's interrupt behavior. Completed reads use the original path.
        if (future.isDone() || !Thread.currentThread().getName().startsWith("DH-World Gen Thread["))
            return future.join();
        try {
            return future.get();
        } catch (InterruptedException shutdown) {
            Thread.currentThread().interrupt();
            // DH recognizes this cause as shutdown. The cached future belongs to other callers too.
            throw new CompletionException(shutdown);
        } catch (ExecutionException | CancellationException failure) {
            // Now completed: retain join's original exception identity and wrapping semantics.
            return future.join();
        }
    }
}
