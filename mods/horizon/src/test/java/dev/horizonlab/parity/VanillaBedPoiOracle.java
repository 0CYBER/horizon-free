package dev.horizonlab.parity;

import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.ai.village.poi.PoiTypes;
import net.minecraft.world.level.block.BedBlock;
import net.minecraft.world.level.block.AbstractBedBlock;

/** Queries the registered POI for every vanilla bed state, including straw. */
public final class VanillaBedPoiOracle {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        try (var out = Files.newBufferedWriter(Path.of(args[0]))) {
            for (var block : BuiltInRegistries.BLOCK) {
                if (!(block instanceof AbstractBedBlock)) continue;
                for (var state : block.getStateDefinition().getPossibleStates()) {
                    boolean home = PoiTypes.forState(state).filter(type -> type.is(PoiTypes.HOME)).isPresent();
                    out.write(BuiltInRegistries.BLOCK.getKey(block) + " " + state.getValue(BedBlock.PART).getSerializedName() + " " + home);
                    out.newLine();
                }
            }
        }
    }
}
