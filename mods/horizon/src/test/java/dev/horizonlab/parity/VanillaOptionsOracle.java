package dev.horizonlab.parity;

import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.*;
import net.minecraft.util.datafix.DataFixers;

/**
 * Loads each options.txt case through the real Options.load and writes it back
 * with Options.save, then dumps the key mapping table (order, defaults,
 * categories). The Minecraft instance is an uninitialised shell that only
 * supplies the data fixer.
 */
public final class VanillaOptionsOracle {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        var uf = u.getDeclaredField("theUnsafe");
        uf.setAccessible(true);
        Object unsafe = uf.get(null);
        var allocate = u.getMethod("allocateInstance", Class.class);
        Minecraft shell = (Minecraft) allocate.invoke(unsafe, Minecraft.class);
        Field fixer = Minecraft.class.getDeclaredField("fixerUpper");
        fixer.setAccessible(true);
        fixer.set(shell, DataFixers.getDataFixer());
        Field instance = Minecraft.class.getDeclaredField("instance");
        instance.setAccessible(true);
        instance.set(null, shell);
        Path cases = Path.of(args[0]);
        Path work = Files.createTempDirectory("opts");
        List<Path> files = new ArrayList<>();
        try (var s = Files.list(cases)) {
            s.sorted().forEach(files::add);
        }
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            boolean first = true;
            for (Path file : files) {
                Path target = work.resolve("options.txt");
                Files.deleteIfExists(target);
                if (!file.getFileName().toString().equals("absent.txt")) {
                    Files.copy(file, target);
                }
                Options options = new Options(shell, work.toFile());
                options.save();
                out.print("== " + file.getFileName() + "\n");
                out.print(Files.readString(target, StandardCharsets.UTF_8));
                if (first) {
                    first = false;
                    out.print("== keys\n");
                    for (KeyMapping k : options.keyMappings) {
                        out.print(k.getName() + " " + k.getDefaultKey().getName() + " " + k.getCategory().id().getPath() + " "
                            + (k instanceof ToggleKeyMapping) + "\n");
                    }
                }
            }
        }
        System.exit(0);
    }
}
