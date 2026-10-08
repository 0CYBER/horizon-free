package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.nio.file.*;import java.util.*;
/** The actual JDK HashMap used by vanilla cooldown storage, with ordered keys. */
public final class VanillaJavaHashMapOracle {
 record Key(int id,int hash) implements Comparable<Key>{@Override public int hashCode(){return hash;}@Override public int compareTo(Key other){return Integer.compare(id,other.id);}}
 static String opt(Integer value){return value==null?"none":value.toString();}
 public static void main(String[] args)throws Exception{Path dir=Path.of(args[1]);try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var keys=new ArrayList<Key>();for(var h:f.getAsJsonArray("hashes"))keys.add(new Key(keys.size(),h.getAsInt()));var map=new HashMap<Key,Integer>();Iterator<Map.Entry<Key,Integer>> cursor=map.entrySet().iterator();for(var ov:f.getAsJsonArray("ops")){var op=ov.getAsJsonArray();int k=op.get(1).getAsInt();try{switch(op.get(0).getAsString()){
 case "put":out.println("old "+opt(map.put(keys.get(k),op.get(2).getAsInt())));break;
 case "remove":out.println("old "+opt(map.remove(keys.get(k))));break;
 case "get":out.println("get "+opt(map.get(keys.get(k))));break;
 case "clear":map.clear();out.println("clear");break;
 case "cursor":cursor=map.entrySet().iterator();out.println("cursor");break;
 case "next":if(cursor.hasNext()){var e=cursor.next();out.println("next "+e.getKey().id()+"="+e.getValue());}else out.println("next none");break;
 case "force-next":{var e=cursor.next();out.println("next "+e.getKey().id()+"="+e.getValue());break;}
 case "delete":cursor.remove();out.println("delete");break;
 default:throw new AssertionError();}}catch(ConcurrentModificationException e){out.println("error modified");}catch(IllegalStateException e){out.println("error current");}catch(NoSuchElementException e){out.println("next none");}
 var entries=new StringJoiner(",");for(var e:map.entrySet())entries.add(e.getKey().id()+"="+e.getValue());out.println("map "+map.size()+" "+map.isEmpty()+" "+cursor.hasNext()+" | "+entries);
 }}}}
}
