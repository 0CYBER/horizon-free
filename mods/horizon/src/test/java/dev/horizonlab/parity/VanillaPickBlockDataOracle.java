package dev.horizonlab.parity;

import com.google.gson.JsonParser;
import com.mojang.logging.LogUtils;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.core.BlockPos;
import net.minecraft.core.component.DataComponentPatch;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.TagParser;
import net.minecraft.resources.Identifier;
import net.minecraft.util.ProblemReporter;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.storage.TagValueOutput;
import org.slf4j.Logger;

/**
 * Original oracle: block entity data on items. Each case line is JSON: a block,
 * the saved NBT of its block entity, the block entity an item is placed as
 * ("target", NBT), the item's component patch ("patch", SNBT; absent: the item
 * Ctrl + Pick Block gave) and whether the player can use game master blocks.
 * Prints the picked item's component patch, a tab, then the placed block
 * entity's saveWithFullMetadata.
 */
public final class VanillaPickBlockDataOracle {
    static final Logger LOGGER = LogUtils.getLogger();
    static final BlockPos POS = new BlockPos(0, 64, 0);

    static BlockEntity load(BlockState state, String snbt) throws Exception {
        BlockEntity entity = BlockEntity.loadStatic(POS, state, TagParser.parseCompoundFully(snbt), ParityData.access);
        if (entity == null) throw new IllegalStateException("no block entity: " + snbt);
        return entity;
    }

    /** ServerGamePacketListenerImpl.addBlockDataToItem on the block's item. */
    static ItemStack pick(BlockState state, BlockEntity entity) {
        ItemStack item = new ItemStack(state.getBlock().asItem());
        try (ProblemReporter.ScopedCollector reporter = new ProblemReporter.ScopedCollector(entity.problemPath(), LOGGER)) {
            TagValueOutput output = TagValueOutput.createWithContext(reporter, ParityData.access);
            entity.saveCustomOnly(output);
            entity.removeComponentsFromTag(output);
            BlockItem.setBlockEntityData(item, entity.getType(), output);
            item.applyComponents(entity.collectComponents());
        }
        return item;
    }

    /** BlockItem.updateCustomBlockEntityTag, then updateBlockEntityComponents. */
    static void place(BlockEntity entity, ItemStack item, boolean gameMaster) {
        var data = item.get(DataComponents.BLOCK_ENTITY_DATA);
        if (data != null && data.type() == entity.getType() && (!entity.getType().onlyOpCanSetNbt() || gameMaster)) {
            data.loadInto(entity, ParityData.access);
        }
        entity.applyComponentsFromItemStack(item);
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                try {
                    BlockState state = BuiltInRegistries.BLOCK.getValue(Identifier.parse(c.get("block").getAsString())).defaultBlockState();
                    ItemStack picked = pick(state, load(state, c.get("nbt").getAsString()));
                    ItemStack item = picked;
                    if (c.has("patch")) {
                        var patch = DataComponentPatch.CODEC.parse(ParityData.ops, TagParser.parseCompoundFully(c.get("patch").getAsString())).getOrThrow();
                        item = new ItemStack(state.getBlock().asItem());
                        item.applyComponents(patch);
                    }
                    BlockEntity target = load(state, c.get("target").getAsString());
                    place(target, item, c.get("op").getAsBoolean());
                    CompoundTag saved = target.saveWithFullMetadata(ParityData.access);
                    var patch = DataComponentPatch.CODEC.encodeStart(ParityData.ops, picked.getComponentsPatch()).getOrThrow();
                    out.println(ParityData.canon(patch) + "\t" + ParityData.canon(saved));
                } catch (Exception e) {
                    System.err.println(line + ": " + e);
                    out.println("error");
                }
            }
        }
    }
}
