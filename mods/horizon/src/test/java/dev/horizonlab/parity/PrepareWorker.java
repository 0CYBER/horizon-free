package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.tags.*;

/** Original launcher transport. Each worker owns one server OR client classpath. */
public final class PrepareWorker {
    private static <T> Runnable restoreTags(Registry<T> registry) {
        var tags = new HashMap<TagKey<T>, List<Holder<T>>>();
        registry.getTags().forEach(tag -> tags.put(tag.key(), tag.stream().toList()));
        return () -> registry.prepareTagReload(new TagLoader.LoadResult<>(registry.key(), tags)).apply();
    }

    public static void main(String[] args) throws Exception {
        if (args.length > 0 && args[0].equals("validate")) {
            try (var paths = Files.walk(Path.of(args[1]))) {
                for (var path : paths.filter(p -> p.toString().endsWith(".class")).toList()) {
                    String name = Path.of(args[1]).relativize(path).toString()
                        .replace(File.separatorChar, '.').replaceAll("\\.class$", "");
                    var type = Class.forName(name, false, PrepareWorker.class.getClassLoader());
                    type.getDeclaredMethods(); type.getDeclaredConstructors(); type.getDeclaredFields();
                }
            }
            return;
        }
        // Bootstrap replaces System.out. Keep the protocol on the original descriptor.
        var replies = new PrintWriter(new OutputStreamWriter(new FileOutputStream(FileDescriptor.out), StandardCharsets.UTF_8), true);
        System.setOut(System.err);
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var resets = new ArrayList<Runnable>();
        for (var registry : BuiltInRegistries.REGISTRY) resets.add(restoreTags(registry));
        System.setProperty("horizon.prepare.reuse", "true");
        try (var input = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8))) {
            for (String line; (line = input.readLine()) != null;) {
                var task = JsonParser.parseString(line).getAsJsonArray();
                var previousOut = System.out; var previousErr = System.err;
                String error = "";
                try (var log = new PrintStream(Files.newOutputStream(Path.of(task.get(0).getAsString()),
                        StandardOpenOption.CREATE, StandardOpenOption.APPEND), true, StandardCharsets.UTF_8)) {
                    System.setOut(log); System.setErr(log);
                    try {
                        var values = task.get(2).getAsJsonArray();
                        var argv = new String[values.size()];
                        for (int i = 0; i < argv.length; i++) argv[i] = values.get(i).getAsString();
                        Class.forName("dev.horizonlab.parity." + task.get(1).getAsString())
                            .getMethod("main", String[].class).invoke(null, (Object) argv);
                    } catch (Throwable failure) {
                        failure.printStackTrace(log); error = failure.toString();
                    } finally {
                        for (var reset : resets) reset.run();
                    }
                } finally {
                    System.setOut(previousOut); System.setErr(previousErr);
                }
                replies.println(new Gson().toJson(error));
                if (!error.isEmpty()) System.exit(1);
            }
        }
        // Vanilla may have started non-daemon executors. All task outputs are closed now.
        System.exit(0);
    }
}
