# Release a blocked terrain read during shutdown

A local world test saved its chunks and closed the DH databases, then hit the
client shutdown watchdog. Its remaining non-daemon DH world-generation worker
was waiting in `IOWorker.isOldChunkAround` on a cached blending-metadata future.
The unchanged control exited normally, so the failure is intermittent; this
observation does not attribute it to the culling optimization.

The scoped redirect uses an interruptible wait only for pending blending reads
on threads named `DH-World Gen Thread[...]`. The pinned DH 3.3.4 pool uses this
name and interrupts remaining workers during shutdown. Ordinary Minecraft
callers keep `CompletableFuture.join` behavior. Completed results use the original
join path. Failed reads retain join's exception identity and wrapping behavior.

An interrupted DH waiter restores its interrupt flag and throws a completion
exception whose interrupted cause is recognized by DH's existing shutdown
handling. It never cancels or completes the cached future, which belongs to other
callers too. Chunk stores, saves, storage closure and world difficulty are untouched.
This changes cancellation of an interrupted background task, rather than terrain
results from an uninterrupted task.

Profiles with distant terrain request the fix; baseline and low-memory profiles
without DH do not. It is restricted to Minecraft 26.3 and DH 3.3.4. To opt out,
set `enabled=false` in `config/horizon-terrain-read-wait.properties` and restart.
Frame reports record `interruptible_terrain_read`, and comparison tools disclose
changes in that state.

`python tools/test_terrain_read.py` checks the actual transformed game IOWorker
inside Fabric, with controlled cached metadata futures. It forces an interrupted
pending DH read, verifies the shared future remains available to a later caller,
and checks that ordinary callers remain blocked until completion. It also
checks an interrupt already set before a pending read, successful completed
reads with an interrupt already set, cancelled
futures, exceptional completion identity, and recognition by DH's shutdown
exception utility. The private fixture does not enter release packs. This is a
correctness check, not an FPS, throughput or Mac shutdown benchmark.
