package dev.horizonlab.geometry;

import com.seibel.distanthorizons.core.pos.DhSectionPos;
import java.util.ArrayList;
import java.util.Random;

/** Differential checks against the actual pinned publisher method, including its unusual rounding. */
public final class RelativeTerrainPositionTest {
    private static int cases, failures;

    private static void check(long position, byte level) {
        cases++;
        long expected;
        try { expected = DhSectionPos.getDhSectionRelativePositionForDetailLevel(position, level); }
        catch (IllegalArgumentException upstream) {
            try { RelativeTerrainPosition.resolve(position, level); }
            catch (IllegalArgumentException patched) {
                if (!upstream.getMessage().equals(patched.getMessage()) || upstream.getClass() != patched.getClass())
                    throw new AssertionError("Publisher exception changed");
                failures++;
                return;
            }
            throw new AssertionError("Invalid level stopped throwing");
        }
        long actual = RelativeTerrainPosition.resolve(position, level);
        if (expected != actual) throw new AssertionError("Relative position changed: " + position + "/" + level);
    }

    public static void main(String[] args) {
        var values = new ArrayList<Integer>();
        values.add(Integer.MIN_VALUE); values.add(Integer.MAX_VALUE); values.add(0);
        for (int bit = 0; bit <= 27; bit++) {
            int boundary = 1 << bit;
            for (int offset = -1; offset <= 1; offset++) {
                values.add(boundary + offset); values.add(-boundary + offset);
            }
        }
        for (int input : new int[] {0, 1, 6, 30, 31, 32, 63, 64, 95, 96, 126, 127}) {
            for (int output = -128; output <= 127; output++) {
                for (int index = 0; index < values.size(); index++) {
                    int x = values.get(index), z = values.get((index * 37 + 13) % values.size());
                    check(DhSectionPos.encode((byte) input, x, z), (byte) output);
                }
            }
        }
        // A direct shift changes this original negative-coordinate result; preserve it exactly.
        check(-7336704456267971543L, (byte) 55);
        Random random = new Random(0x6f9d1413);
        for (int index = 0; index < 1000000; index++) check(random.nextLong(), (byte) random.nextInt(256));
        System.out.println("Actual publisher position differential checks passed: cases=" + cases + " exceptions=" + failures);
    }
}
