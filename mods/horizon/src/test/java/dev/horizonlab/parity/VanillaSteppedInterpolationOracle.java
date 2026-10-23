package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;
import net.minecraft.core.PositionAndRotation;
import net.minecraft.world.entity.PositionPath;
import net.minecraft.world.entity.PositionStep;
import net.minecraft.world.phys.Vec3;

/**
 * How a client walks a living entity along the positions its server sends: the unmodified
 * {@code SteppedInterpolationHandler.InterpolationData} (setStartingPoint, addStep, addSteps,
 * getNewPositionAndRotation, advance, reset), driven the way
 * {@code AbstractInterpolationHandler.interpolateTo} / {@code interpolate} and
 * {@code SteppedInterpolationHandler.startInterpolating} / {@code doInterpolate} drive it, with the
 * entity's own {@code setPos} / {@code setRot}. The handler's entity hooks are out of scope: the
 * entity moves only by its interpolation (no deltas to adjust) at the normal tick rate.
 *
 * <p>Input (args[0]), one event a line: {@code case STEPS X Y Z YAW PITCH} (a fresh entity there
 * with a handler of that many interpolation steps), {@code move L X Y Z} or {@code move S N
 * (X Y Z TICKS)xN} or {@code move N} (no position), then {@code R YAW PITCH} or {@code -} (no
 * rotation), and {@code tick}. Output (args[1]): the entity after every event.
 */
public final class VanillaSteppedInterpolationOracle {
    private static Method method(Class<?> owner, String name, Class<?>... types) throws Exception {
        Method m = owner.getDeclaredMethod(name, types);
        m.setAccessible(true);
        return m;
    }

    private static String hex(double v) {
        return Long.toHexString(Double.doubleToRawLongBits(v));
    }

    private static String hex(float v) {
        return Integer.toHexString(Float.floatToRawIntBits(v));
    }

    public static void main(String[] args) throws Exception {
        Class<?> dataClass = Class.forName("net.minecraft.world.entity.SteppedInterpolationHandler$InterpolationData");
        Constructor<?> create = dataClass.getDeclaredConstructor();
        create.setAccessible(true);
        Method advance = method(dataClass, "advance", float.class, int.class);
        Method addStep = method(dataClass, "addStep", Vec3.class, float.class, float.class, int.class);
        Method addSteps = method(dataClass, "addSteps", PositionPath.class, float.class, float.class, int.class);
        Method setStartingPoint = method(dataClass, "setStartingPoint", Vec3.class, float.class, float.class);
        Method next = method(dataClass, "getNewPositionAndRotation");
        Method reset = method(dataClass, "reset");
        Field remaining = dataClass.getDeclaredField("remainingSteps");
        remaining.setAccessible(true);

        Object data = null;
        int steps = 0;
        Vec3 pos = Vec3.ZERO;
        float yRot = 0;
        float xRot = 0;
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))) {
            for (String line : Files.readAllLines(Path.of(args[0]))) {
                String[] f = line.trim().split(" ");
                int i = 1;
                String what;
                switch (f[0]) {
                    case "case" -> {
                        data = create.newInstance();
                        steps = Integer.parseInt(f[1]);
                        pos = new Vec3(Double.parseDouble(f[2]), Double.parseDouble(f[3]), Double.parseDouble(f[4]));
                        yRot = Float.parseFloat(f[5]);
                        xRot = Float.parseFloat(f[6]);
                        what = "c";
                    }
                    case "move" -> {
                        PositionPath path = null;
                        switch (f[i++]) {
                            case "L" -> {
                                path = PositionPath.of(new Vec3(Double.parseDouble(f[i]), Double.parseDouble(f[i + 1]), Double.parseDouble(f[i + 2])));
                                i += 3;
                            }
                            case "S" -> {
                                int n = Integer.parseInt(f[i++]);
                                List<PositionStep> list = new ArrayList<>();
                                for (int k = 0; k < n; k++) {
                                    list.add(new PositionStep(new Vec3(Double.parseDouble(f[i]), Double.parseDouble(f[i + 1]), Double.parseDouble(f[i + 2])), Integer.parseInt(f[i + 3])));
                                    i += 4;
                                }
                                path = PositionPath.stepped(List.copyOf(list));
                            }
                            default -> {
                            }
                        }
                        boolean hasRotation = f[i].equals("R");
                        float y = hasRotation ? Float.parseFloat(f[i + 1]) : 0;
                        float x = hasRotation ? Float.parseFloat(f[i + 2]) : 0;
                        boolean active = !((List<?>) remaining.get(data)).isEmpty();
                        PositionAndRotation.Mutable target = (PositionAndRotation.Mutable) data;
                        // AbstractInterpolationHandler.interpolateTo(path, yRot, xRot, hasRotation).
                        PositionAndRotation current = active ? target : PositionAndRotation.of(pos, yRot, xRot);
                        if (path == null) path = PositionPath.of(current.position());
                        if (!hasRotation) {
                            y = current.yRot();
                            x = current.xRot();
                        }
                        Vec3 end = path.endPosition();
                        if (steps == 0) {
                            // snapTo, then cancel.
                            pos = end;
                            yRot = y % 360.0F;
                            xRot = Math.clamp(x % 360.0F % 360.0F, -90.0F, 90.0F);
                            reset.invoke(data);
                            what = "s";
                        } else if (active && target.is(end, y, x)) {
                            what = "k";
                        } else {
                            // SteppedInterpolationHandler.startInterpolating.
                            if (!active) setStartingPoint.invoke(data, pos, yRot, xRot);
                            if (Objects.equals(end, target.position())) addStep.invoke(data, end, y, x, steps);
                            else addSteps.invoke(data, path, y, x, steps);
                            target.set(end, y, x);
                            what = "i";
                        }
                    }
                    case "tick" -> {
                        // AbstractInterpolationHandler.interpolate.
                        if (((List<?>) remaining.get(data)).isEmpty()) {
                            reset.invoke(data);
                            what = "n";
                        } else {
                            PositionAndRotation t = (PositionAndRotation) next.invoke(data);
                            pos = t.position();
                            yRot = t.yRot() % 360.0F;
                            xRot = Math.clamp(t.xRot() % 360.0F % 360.0F, -90.0F, 90.0F);
                            advance.invoke(data, 1.0F, steps);
                            what = "t";
                        }
                    }
                    default -> throw new IllegalArgumentException(line);
                }
                out.println(what + " " + hex(pos.x) + " " + hex(pos.y) + " " + hex(pos.z) + " " + hex(yRot) + " " + hex(xRot));
            }
        }
    }
}
