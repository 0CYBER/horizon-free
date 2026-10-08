package dev.horizonlab.parity;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.ticks.*;

/** Original transport for native proto tick storage and world-generation routing. */
public final class VanillaGenerationTickOracle {
 private static final Object[] TYPES=new Object[32];
 static{for(int i=0;i<TYPES.length;i++)TYPES[i]=new Object();}
 private static final class Stop extends RuntimeException{}
 private static final class Input {
  final String[] words;int cursor=1;Input(String line){words=line.split(" ");}
  int integer(){return Integer.parseInt(words[cursor++]);}long number(){return Long.parseLong(words[cursor++]);}
  BlockPos pos(){return new BlockPos(integer(),integer(),integer());}long chunk(){return new ChunkPos(integer(),integer()).pack();}
  SavedTick<Object> saved(){return new SavedTick<>(TYPES[integer()],pos(),integer(),TickPriority.byValue(integer()));}
 }
 private final Map<Long,TickContainerAccess<Object>> chunks=new HashMap<>();
 private final PrintWriter out;private boolean fail;private List<SavedTick<Object>> snapshot=List.of();
 private final WorldGenTickAccess<Object> access;
 private VanillaGenerationTickOracle(PrintWriter out){this.out=out;access=new WorldGenTickAccess<>(p->{out.println("lookup "+p.getX()+" "+p.getY()+" "+p.getZ());if(fail)throw new Stop();return chunks.computeIfAbsent(ChunkPos.pack(p),key->new ProtoChunkTicks<>());});}
 private static int type(Object value){for(int i=0;i<TYPES.length;i++)if(TYPES[i]==value)return i;throw new AssertionError();}
 private void emit(List<SavedTick<Object>> ticks){out.println("saved "+ticks.size());for(var t:ticks)out.println("value "+type(t.type())+" "+t.pos().getX()+" "+t.pos().getY()+" "+t.pos().getZ()+" "+t.delay()+" "+t.priority().getValue());}
 @SuppressWarnings("unchecked") private void command(String line,int index){var in=new Input(line);String op=in.words[0];out.println("command "+index+" "+op);
  try{switch(op){
   case "reset"->{chunks.clear();snapshot=List.of();fail=false;}
   case "fail"->fail=in.integer()!=0;
   case "load","level"->{long key=in.chunk();int count=in.integer();var ticks=new ArrayList<SavedTick<Object>>();for(int i=0;i<count;i++)ticks.add(in.saved());chunks.put(key,op.equals("load")?ProtoChunkTicks.load(ticks):new LevelChunkTicks<>(ticks));}
   case "schedule"->{var t=new ScheduledTick<>(TYPES[in.integer()],in.pos(),in.number(),TickPriority.byValue(in.integer()),in.number());access.schedule(t);out.println("scheduled");}
   case "query"->{var type=TYPES[in.integer()];var p=in.pos();out.println("query "+access.hasScheduledTick(p,type));}
   case "will"->{var type=TYPES[in.integer()];var p=in.pos();out.println("will "+access.willTickThisTick(p,type));}
   case "dump"->{var c=chunks.get(in.chunk());long time=in.number();if(c==null)out.println("absent");else emit(((SerializableTickContainer<Object>)c).pack(time));}
   case "snapshot"->{var c=chunks.get(in.chunk());snapshot=c instanceof ProtoChunkTicks<?>?((ProtoChunkTicks<Object>)c).scheduledTicks():List.of();}
   case "snapshot_dump"->emit(snapshot);
   case "unpack","promote"->{long key=in.chunk(),time=in.number();var c=chunks.get(key);if(op.equals("promote")&&c instanceof ProtoChunkTicks<?>){c=new LevelChunkTicks<>(((ProtoChunkTicks<Object>)c).scheduledTicks());chunks.put(key,c);}if(c instanceof LevelChunkTicks<?>)((LevelChunkTicks<Object>)c).unpack(time);}
   case "remove"->chunks.remove(in.chunk());
   case "count"->{}
   default->throw new AssertionError(op);
  }}catch(Stop ignored){out.println("error");}
  if(in.cursor!=in.words.length)throw new AssertionError("extra input");out.println("count "+access.count());
 }
 public static void main(String[] args)throws Exception{
  net.minecraft.SharedConstants.tryDetectVersion();
  try(var input=Files.newBufferedReader(Path.of(args[0]));var out=new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))){var oracle=new VanillaGenerationTickOracle(out);String line;int index=0;while((line=input.readLine())!=null)oracle.command(line,index++);}
 }
}
