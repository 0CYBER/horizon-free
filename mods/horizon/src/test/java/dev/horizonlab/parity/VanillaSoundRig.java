package dev.horizonlab.parity;

import com.mojang.blaze3d.audio.*;
import java.io.PrintWriter;
import java.lang.reflect.*;
import java.nio.ByteBuffer;
import java.nio.file.Path;
import java.util.*;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.Executor;
import javax.sound.sampled.AudioFormat;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.resources.sounds.SoundInstance;
import net.minecraft.client.sounds.*;
import net.minecraft.network.chat.contents.TranslatableContents;
import net.minecraft.resources.Identifier;
import net.minecraft.sounds.SoundSource;
import org.lwjgl.openal.AL10;

/**
 * A real vanilla SoundManager/SoundEngine running on OpenAL Soft's "No Output"
 * device (ALSOFT_DRIVERS=null), shared by the audio oracles.
 *
 * Determinism: the sound executor runs each task inline (the model where the
 * sound thread keeps up between client ticks); static sounds get one silent
 * hour-long buffer (they play until stopped or "finished" by the script) and
 * streams play endless silence. Everything else, the
 * channel pools, AL source parameters, lifetimes and bookkeeping, is the real
 * code and the real OpenAL implementation.
 */
public final class VanillaSoundRig {
    static Object unsafe;
    static Method allocate;

    static <T> T make(Class<T> type) throws Exception {
        if (unsafe == null) {
            var u = Class.forName("sun.misc.Unsafe");
            unsafe = field(u, "theUnsafe").get(null);
            allocate = u.getMethod("allocateInstance", Class.class);
        }
        return type.cast(allocate.invoke(unsafe, type));
    }

    static Field field(Class<?> type, String name) throws Exception {
        var f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static Object get(Object target, Class<?> type, String name) throws Exception {
        return field(type, name).get(target);
    }

    static void set(Object target, Class<?> type, String name, Object value) throws Exception {
        field(type, name).set(target, value);
    }

    /** Runs every task on the calling thread. */
    public static final class Inline extends SoundEngineExecutor {
        @Override
        public void execute(final Runnable command) {
            command.run();
        }

        @Override
        public void shutDown() {
        }

        @Override
        public void startUp() {
        }
    }

    static SoundBuffer silence;

    /** Static sounds: a silent hour; streams: endless silence. */
    public static final class Buffers extends SoundBufferLibrary {
        Buffers() {
            super(null);
        }

        @Override
        public CompletableFuture<SoundBuffer> getCompleteBuffer(final Identifier location) {
            return CompletableFuture.completedFuture(silence);
        }

        @Override
        public CompletableFuture<AudioStream> getStream(final Identifier location, final boolean looping) {
            return CompletableFuture.completedFuture(new Silence());
        }
    }

    /** An endless silent stream, a minute per read (it never runs dry between ticks). */
    public static final class Silence implements AudioStream {
        static final AudioFormat FORMAT = new AudioFormat(8000.0F, 8, 1, false, false);

        @Override
        public AudioFormat getFormat() {
            return FORMAT;
        }

        @Override
        public ByteBuffer read(final int expectedSize) {
            ByteBuffer data = ByteBuffer.allocateDirect(8000 * 60);
            for (int i = 0; i < data.capacity(); i++) data.put(i, (byte) 0x80);
            return data;
        }

        @Override
        public void close() {
        }
    }

    public final SoundManager manager;
    public final SoundEngine engine;
    public final Options options;
    public final List<String> heard = new ArrayList<>();

    public VanillaSoundRig(Path root) throws Exception {
        manager = VanillaSoundEventOracle.load(root);
        engine = (SoundEngine) get(manager, SoundManager.class, "soundEngine");
        options = (Options) get(engine, SoundEngine.class, "options");
        var inline = make(Inline.class);
        set(engine, SoundEngine.class, "executor", inline);
        set(get(engine, SoundEngine.class, "channelAccess"), ChannelAccess.class, "executor", inline);
        set(engine, SoundEngine.class, "soundBuffers", make(Buffers.class));
        Library library = (Library) get(engine, SoundEngine.class, "library");
        library.init(null, DeviceList.query(), false);
        ByteBuffer data = ByteBuffer.allocateDirect(8000 * 3600);
        for (int i = 0; i < data.capacity(); i++) data.put(i, (byte) 0x80);
        silence = new SoundBuffer(data, new AudioFormat(8000.0F, 8, 1, false, false));
        set(engine, SoundEngine.class, "loaded", true);
        Method slider = Options.class.getDeclaredMethod("createSoundSliderOptionInstance", String.class, SoundSource.class);
        slider.setAccessible(true);
        var volumes = new EnumMap<SoundSource, OptionInstance<Double>>(SoundSource.class);
        for (SoundSource source : SoundSource.values()) {
            volumes.put(source, (OptionInstance<Double>) slider.invoke(options, "soundCategory." + source.getName(), source));
        }
        set(options, Options.class, "soundSourceVolumes", volumes);
        manager.addListener((sound, event, range) -> {
            String subtitle = "-";
            if (event.getSubtitle() != null && event.getSubtitle().getContents() instanceof TranslatableContents t) subtitle = t.getKey();
            heard.add(String.format("notify %s %s %08x %s %016x %016x %016x", sound.getIdentifier(), subtitle, Float.floatToRawIntBits(range),
                sound.isLooping(), Double.doubleToRawLongBits(sound.getX()), Double.doubleToRawLongBits(sound.getY()),
                Double.doubleToRawLongBits(sound.getZ())));
        });
    }

    /** Set a sound source volume option without its change callback. */
    public void volume(SoundSource source, double value) throws Exception {
        set(options.getSoundSourceOptionInstance(source), OptionInstance.class, "value", value);
    }

    /** The channel of an instance, or null. */
    @SuppressWarnings("unchecked")
    public Channel channel(SoundInstance instance) throws Exception {
        var map = (Map<SoundInstance, ChannelAccess.ChannelHandle>) get(engine, SoundEngine.class, "instanceToChannel");
        var handle = map.get(instance);
        return handle == null ? null : (Channel) get(handle, ChannelAccess.ChannelHandle.class, "channel");
    }

    /** The channel's end: what OpenAL does when a non-looping buffer runs out. */
    public void finish(SoundInstance instance) throws Exception {
        var channel = channel(instance);
        if (channel != null) channel.stop();
    }

    static String fbits(float v) {
        return String.format("%08x", Float.floatToRawIntBits(v));
    }

    /** One line describing an instance: activity and its AL source. */
    public String describe(String name, SoundInstance instance) throws Exception {
        StringBuilder sb = new StringBuilder("  ").append(name).append(" active=").append(manager.isActive(instance));
        var channel = channel(instance);
        if (channel == null) {
            sb.append(" channel=-");
        } else {
            int source = (int) get(channel, Channel.class, "source");
            boolean initialized = ((java.util.concurrent.atomic.AtomicBoolean) get(channel, Channel.class, "initialized")).get();
            if (!initialized) {
                sb.append(" channel=released");
            } else {
                float[] pos = new float[3];
                AL10.alGetSourcefv(source, AL10.AL_POSITION, pos);
                sb.append(String.format(" state=%x gain=%s pitch=%s pos=%s,%s,%s rel=%d loop=%d model=%x max=%s rolloff=%s ref=%s",
                    AL10.alGetSourcei(source, AL10.AL_SOURCE_STATE), fbits(AL10.alGetSourcef(source, AL10.AL_GAIN)),
                    fbits(AL10.alGetSourcef(source, AL10.AL_PITCH)), fbits(pos[0]), fbits(pos[1]), fbits(pos[2]),
                    AL10.alGetSourcei(source, AL10.AL_SOURCE_RELATIVE), AL10.alGetSourcei(source, AL10.AL_LOOPING),
                    AL10.alGetSourcei(source, 0xD000), fbits(AL10.alGetSourcef(source, AL10.AL_MAX_DISTANCE)),
                    fbits(AL10.alGetSourcef(source, AL10.AL_ROLLOFF_FACTOR)), fbits(AL10.alGetSourcef(source, AL10.AL_REFERENCE_DISTANCE))));
            }
        }
        return sb.toString();
    }

    /** Print and clear the notifications, then the channel counts. */
    public void flush(PrintWriter out) {
        for (String h : heard) out.println(h);
        heard.clear();
        out.println("  " + manager.getChannelDebugString());
    }
}
