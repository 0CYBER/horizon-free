package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.classfile.*;
import java.lang.classfile.instruction.InvokeInstruction;
import java.lang.constant.ClassDesc;
import java.lang.constant.MethodTypeDesc;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

/**
 * Runs the real FramerateLimiter.limitDisplayFPS from the client jar with its
 * clock calls (System.nanoTime, LockSupport.parkNanos, Thread.interrupted,
 * Thread.onSpinWait) redirected by a ClassFile rewrite to a scripted virtual
 * clock, and the Minecraft.runTick condition (limits below 260), printing the
 * limiter state and clock interaction of every frame.
 */
public final class VanillaFrameLimiterOracle {
    /** The scripted clock the rewritten limiter calls. */
    public static final class Clock {
        static long now, call, spin;
        static long[] overshoot;
        static int nextOvershoot;
        static boolean[] interrupts;
        static int nextInterrupt;
        static List<Long> parks = new ArrayList<>();
        static long spins, queries;

        public static long nanoTime() {
            queries++;
            long t = now;
            now += call;
            return t;
        }

        public static void parkNanos(long nanos) {
            parks.add(nanos);
            long o = overshoot[nextOvershoot++ % overshoot.length];
            now += Math.max(Math.max(nanos, 0) + o, 1);
        }

        public static void onSpinWait() {
            spins++;
            now += spin;
        }

        public static boolean interrupted() {
            return interrupts[nextInterrupt++ % interrupts.length];
        }
    }

    static final ClassDesc CLOCK = ClassDesc.of(Clock.class.getName());

    /** Loads FramerateLimiter itself, rewritten; everything else from the parent. */
    static final class Rewriting extends ClassLoader {
        Rewriting(ClassLoader parent) {
            super(parent);
        }

        @Override
        protected Class<?> loadClass(String name, boolean resolve) throws ClassNotFoundException {
            if (!name.equals("net.minecraft.client.FramerateLimiter")) {
                return super.loadClass(name, resolve);
            }
            synchronized (getClassLoadingLock(name)) {
                Class<?> loaded = findLoadedClass(name);
                if (loaded == null) {
                    try (var in = getParent().getResourceAsStream("net/minecraft/client/FramerateLimiter.class")) {
                        byte[] bytes = rewrite(in.readAllBytes());
                        loaded = defineClass(name, bytes, 0, bytes.length);
                    } catch (IOException e) {
                        throw new ClassNotFoundException(name, e);
                    }
                }
                return loaded;
            }
        }
    }

    static int rewrites;

    static byte[] rewrite(byte[] bytes) {
        var cf = ClassFile.of();
        ClassModel model = cf.parse(bytes);
        return cf.transformClass(model, ClassTransform.transformingMethodBodies((builder, element) -> {
            if (element instanceof InvokeInstruction call && call.opcode() == Opcode.INVOKESTATIC) {
                String owner = call.owner().asInternalName();
                String name = call.name().stringValue();
                String target = switch (owner + "." + name) {
                    case "java/lang/System.nanoTime" -> "nanoTime";
                    case "java/util/concurrent/locks/LockSupport.parkNanos" -> "parkNanos";
                    case "java/lang/Thread.interrupted" -> "interrupted";
                    case "java/lang/Thread.onSpinWait" -> "onSpinWait";
                    default -> null;
                };
                if (target != null) {
                    rewrites++;
                    builder.invokestatic(CLOCK, target, MethodTypeDesc.ofDescriptor(call.type().stringValue()));
                    return;
                }
            }
            builder.with(element);
        }));
    }

    public static void main(String[] args) throws Exception {
        Clock.overshoot = new long[] {0};
        Clock.interrupts = new boolean[] {false};
        Class<?> limiter = Class.forName("net.minecraft.client.FramerateLimiter", true,
            new Rewriting(VanillaFrameLimiterOracle.class.getClassLoader()));
        // System.nanoTime x4 (and once in the static initialiser), parkNanos, interrupted, onSpinWait.
        if (rewrites != 8) {
            throw new AssertionError("unexpected clock calls in FramerateLimiter: " + rewrites);
        }
        Method limit = limiter.getMethod("limitDisplayFPS", int.class);
        Field last = limiter.getDeclaredField("lastFrameTime");
        Field average = limiter.getDeclaredField("averageOvershootNs");
        Field lastLimit = limiter.getDeclaredField("lastFramerateLimit");
        for (Field f : List.of(last, average, lastLimit)) {
            f.setAccessible(true);
        }
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            int n = 0;
            for (var line : (Iterable<String>) lines::iterator) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                out.println("== " + n++);
                Clock.now = c.get("now").getAsLong();
                Clock.call = c.get("call").getAsLong();
                Clock.spin = c.get("spin").getAsLong();
                var o = c.getAsJsonArray("overshoot");
                Clock.overshoot = new long[o.size()];
                for (int i = 0; i < o.size(); i++) {
                    Clock.overshoot[i] = o.get(i).getAsLong();
                }
                var in = c.getAsJsonArray("interrupts");
                Clock.interrupts = new boolean[in.size()];
                for (int i = 0; i < in.size(); i++) {
                    Clock.interrupts[i] = in.get(i).getAsBoolean();
                }
                Clock.nextOvershoot = 0;
                Clock.nextInterrupt = 0;
                last.setLong(null, c.get("last").getAsLong());
                average.setLong(null, c.get("average").getAsLong());
                lastLimit.setInt(null, c.get("last_limit").getAsInt());
                for (var e : c.getAsJsonArray("frames")) {
                    var frame = e.getAsJsonArray();
                    Clock.now += frame.get(0).getAsLong();
                    Clock.parks.clear();
                    Clock.spins = 0;
                    Clock.queries = 0;
                    int framerateLimit = frame.get(1).getAsInt();
                    // Minecraft.runTick: "if (framerateLimit < 260) FramerateLimiter.limitDisplayFPS(framerateLimit)".
                    if (framerateLimit < 260) {
                        limit.invoke(null, framerateLimit);
                    }
                    var parks = new StringJoiner(",");
                    for (long p : Clock.parks) {
                        parks.add(Long.toString(p));
                    }
                    out.println("frame " + Clock.now + " " + last.getLong(null) + " " + average.getLong(null) + " "
                        + lastLimit.getInt(null) + " queries " + Clock.queries + " spins " + Clock.spins + " parks [" + parks + "]");
                }
            }
        }
        System.exit(0);
    }
}
