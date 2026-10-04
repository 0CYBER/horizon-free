package dev.horizonlab.geometry;

import com.seibel.distanthorizons.core.pos.DhSectionPos;

/** Numeric-only conversion; preserve the pinned library's truncation and negative-coordinate bias. */
public final class RelativeTerrainPosition {
    private RelativeTerrainPosition() {}

    public static long resolve(long position, byte outputLevel) {
        int delta = outputLevel - DhSectionPos.getDetailLevel(position);
        int shift = delta & 31;
        // Invalid levels keep the publisher exception; a sign-bit divisor keeps its original arithmetic.
        if (delta < 0 || shift == 31) {
            return DhSectionPos.getDhSectionRelativePositionForDetailLevel(position, outputLevel);
        }
        int divisorMask = (1 << shift) - 1;
        int negativeBias = divisorMask | ((divisorMask - 1) >>> 31);
        return DhSectionPos.encode(outputLevel,
            coordinate(DhSectionPos.getX(position), shift, divisorMask, negativeBias),
            coordinate(DhSectionPos.getZ(position), shift, divisorMask, negativeBias));
    }

    private static int coordinate(int value, int shift, int divisorMask, int negativeBias) {
        int negative = value >> 31;
        int adjusted = value + (negative & negativeBias);
        // Adding the divisor mask only for a negative adjusted value gives truncation toward zero.
        int truncated = (adjusted + ((adjusted >> 31) & divisorMask)) >> shift;
        return (truncated + negative) & 63;
    }
}
