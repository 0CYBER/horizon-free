package dev.horizonlab.metrics;
import java.lang.instrument.Instrumentation;
public final class ObjectSizeAgent {
 private static Instrumentation instrumentation;
 public static void premain(String args, Instrumentation value) { instrumentation=value; }
 public static long size(Object value) { return instrumentation.getObjectSize(value); }
}
