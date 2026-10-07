package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.nio.file.*;import java.util.*;import it.unimi.dsi.fastutil.objects.ObjectOpenHashSet;
/** Actual default fastutil set used for pending attribute updates. */
public final class VanillaObjectOpenSetOracle {
 record Key(int id,int hash,int object){@Override public int hashCode(){return hash;}@Override public boolean equals(Object o){return o instanceof Key k&&id==k.id;}}
 static String desc(Key key){return key==null?"null":key.id()+":"+key.object();}
 public static void main(String[] args)throws Exception{Path dir=Path.of(args[1]);try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var keys=new ArrayList<Key>();for(var h:f.getAsJsonArray("hashes")){int i=keys.size();keys.add(new Key(f.getAsJsonArray("ids").get(i).getAsInt(),h.getAsInt(),i));}var set=new ObjectOpenHashSet<Key>();Iterator<Key> cursor=set.iterator();for(var ov:f.getAsJsonArray("ops")){var op=ov.getAsJsonArray();int index=op.get(1).getAsInt();Key key=index<0?null:keys.get(index);try{switch(op.get(0).getAsString()){
 case "add"->out.println("add "+set.add(key));case "remove"->out.println("remove "+set.remove(key));case "contains"->out.println("contains "+set.contains(key));case "get"->out.println("get "+desc(set.get(key)));case "clear"->{set.clear();out.println("clear");}case "cursor"->{cursor=set.iterator();out.println("cursor");}case "next"->{if(cursor.hasNext())out.println("next "+desc(cursor.next()));else out.println("next none");}case "force-next"->out.println("next "+desc(cursor.next()));case "delete"->{cursor.remove();out.println("delete");}default->throw new AssertionError();}
 }catch(IllegalStateException e){out.println("error current");}catch(NoSuchElementException e){out.println("next none");}catch(NullPointerException e){out.println("error null");}catch(IndexOutOfBoundsException e){out.println("error bounds");}
 var entries=new StringJoiner(",");set.forEach(k->entries.add(desc(k)));out.println("set "+set.size()+" "+set.isEmpty()+" "+cursor.hasNext()+" | "+entries);
 }}}}
}
