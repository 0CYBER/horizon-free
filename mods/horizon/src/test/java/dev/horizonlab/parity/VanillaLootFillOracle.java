package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.ReloadableServerRegistries;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.util.RandomSource;
import net.minecraft.util.context.ContextMap;
import net.minecraft.world.SimpleContainer;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.storage.loot.LootContext;
import net.minecraft.world.level.storage.loot.LootParams;
import net.minecraft.world.level.storage.loot.LootTable;
import net.minecraft.world.level.storage.loot.parameters.LootContextParamSets;

/**
 * Original oracle: LootTable.fill for container loot tables and seeds, as
 * RandomizableContainer.unpackLootTable does with no luck and no origin
 * (location checks fail). The level is a bare stand-in that only answers
 * registryAccess; every item counts as enabled. Tables whose functions need
 * a real level (exploration maps) print an error line instead.
 */
public final class VanillaLootFillOracle {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();Bootstrap.bootStrap();
        Path out=Path.of(args[0]);
        var repo=ServerPacksSource.createVanillaTrustedRepository();repo.reload();repo.setSelected(List.of("vanilla"));
        var resources=new MultiPackResourceManager(PackType.SERVER_DATA,repo.openAllSelected());
        var layers=RegistryLayer.createRegistryAccess();
        var staticTags=TagLoader.loadTagsForExistingRegistries(resources,layers.getLayer(RegistryLayer.STATIC));
        var context=TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD),staticTags);
        RegistryAccess.Frozen world=RegistryDataLoader.load(resources,context,RegistryDataLoader.WORLD_REGISTRIES,Runnable::run).join();
        var worldLayers=layers.replaceFrom(RegistryLayer.WORLD,world);
        var loaded=ReloadableServerRegistries.reload(worldLayers,staticTags,resources,Runnable::run).join();
        HolderLookup.Provider lookup=loaded.lookupWithUpdatedTags();
        var components=BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(lookup);
        staticTags.forEach(Registry.PendingTags::apply);
        components.forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
        RegistryAccess access=loaded.layers().compositeAccess();
        var unsafeField=Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");unsafeField.setAccessible(true);
        Object unsafe=unsafeField.get(null);
        ServerLevel level=(ServerLevel)unsafe.getClass().getMethod("allocateInstance",Class.class).invoke(unsafe,ServerLevel.class);
        Field registryAccess=Level.class.getDeclaredField("registryAccess");registryAccess.setAccessible(true);
        long offset=(long)unsafe.getClass().getMethod("objectFieldOffset",Field.class).invoke(unsafe,registryAccess);
        unsafe.getClass().getMethod("putObject",Object.class,long.class,Object.class).invoke(unsafe,level,offset,access);
        var tables=lookup.lookupOrThrow(Registries.LOOT_TABLE);
        List<String> names=new ArrayList<>();
        if(args[1].equals("chests"))tables.listElementIds().map(k->k.identifier().toString()).filter(n->n.startsWith("minecraft:chests/")).sorted().forEach(names::add);
        else names.addAll(Arrays.asList(args[1].split(",")));
        Constructor<LootContext> ctor=LootContext.class.getDeclaredConstructor(LootParams.class,RandomSource.class,HolderGetter.Provider.class);ctor.setAccessible(true);
        Method slots=LootTable.class.getDeclaredMethod("getAvailableSlots",net.minecraft.world.Container.class,RandomSource.class);slots.setAccessible(true);
        Method split=LootTable.class.getDeclaredMethod("shuffleAndSplitItems",it.unimi.dsi.fastutil.objects.ObjectArrayList.class,int.class,RandomSource.class);split.setAccessible(true);
        try(var w=new PrintWriter(Files.newBufferedWriter(out,StandardCharsets.UTF_8))) {
            for(String name:names) {
                LootTable table=tables.getOrThrow(ResourceKey.create(Registries.LOOT_TABLE,Identifier.parse(name))).value();
                for(int i=2;i<args.length;i++) {
                    long seed=Long.parseLong(args[i]);
                    w.println("table "+name+" "+seed);
                    try {
                        var params=new LootParams(level,ContextMap.EMPTY,Map.of(),0.0F);
                        LootContext lootContext=ctor.newInstance(params,RandomSource.create(seed),lookup);
                        var items=new it.unimi.dsi.fastutil.objects.ObjectArrayList<ItemStack>();
                        table.getRandomItemsRaw(lootContext,result->{
                            if(result.getCount()<result.getMaxStackSize())items.add(result);
                            else{int count=result.getCount();while(count>0){ItemStack copy=result.copyWithCount(Math.min(result.getMaxStackSize(),count));count-=copy.getCount();items.add(copy);}}
                        });
                        var container=new SimpleContainer(27);
                        RandomSource random=lootContext.getRandom();
                        @SuppressWarnings("unchecked") List<Integer> available=(List<Integer>)slots.invoke(table,container,random);
                        split.invoke(table,items,available.size(),random);
                        for(ItemStack stack:items){if(available.isEmpty())break;container.setItem(available.remove(available.size()-1),stack);}
                        for(int slot=0;slot<27;slot++) {
                            ItemStack stack=container.getItem(slot);
                            if(stack.isEmpty())continue;
                            StringBuilder line=new StringBuilder("fill "+name+" "+seed+" "+slot+" "+BuiltInRegistries.ITEM.getKey(stack.getItem())+" "+stack.getCount()+" "+stack.getDamageValue());
                            var enchantments=stack.getOrDefault(DataComponents.ENCHANTMENTS,net.minecraft.world.item.enchantment.ItemEnchantments.EMPTY);
                            var stored=stack.getOrDefault(DataComponents.STORED_ENCHANTMENTS,net.minecraft.world.item.enchantment.ItemEnchantments.EMPTY);
                            for(var e:enchantments.entrySet())line.append(" "+e.getKey().unwrapKey().get().identifier()+"="+e.getIntValue());
                            for(var e:stored.entrySet())line.append(" "+e.getKey().unwrapKey().get().identifier()+"="+e.getIntValue());
                            w.println(line);
                        }
                    } catch(Exception e) {
                        Throwable cause=e instanceof InvocationTargetException ite?ite.getCause():e;
                        w.println("error "+name+" "+seed+" "+cause);
                    }
                }
            }
        }
        System.exit(0);
    }
}
