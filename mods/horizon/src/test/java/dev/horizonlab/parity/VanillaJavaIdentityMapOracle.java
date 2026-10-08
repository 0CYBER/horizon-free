package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.nio.file.*;import java.util.*;
/** Actual JDK map copies and mixed comparable/non-comparable collision bins. */
public final class VanillaJavaIdentityMapOracle {
 static abstract class Key {final int object,id,hash;Key(int o,int id,int h){this.object=o;this.id=id;hash=h;}@Override public int hashCode(){return hash;}@Override public boolean equals(Object o){return o!=null&&getClass()==o.getClass()&&id==((Key)o).id;}}
 static final class A extends Key{A(int o,int i,int h){super(o,i,h);}}
 static final class B extends Key{B(int o,int i,int h){super(o,i,h);}}
 static final class C extends Key implements Comparable<C>{C(int o,int i,int h){super(o,i,h);}public int compareTo(C other){return Integer.compare(id%4,other.id%4);}}
 static final class D extends Key implements Comparable<D>{D(int o,int i,int h){super(o,i,h);}public int compareTo(D other){return Integer.compare(id,other.id);}}
 static String opt(Integer value){return value==null?"none":value.toString();}
 public static void main(String[] args)throws Exception{Path dir=Path.of(args[1]);try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var enriched=new PrintWriter(Files.newBufferedWriter(dir.resolve("inputs-"+args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var keys=new ArrayList<Key>();var identities=new JsonArray();for(var kv:f.getAsJsonArray("keys")){var k=kv.getAsJsonObject();int o=keys.size(),i=k.get("id").getAsInt(),h=k.get("hash").getAsInt();Key key=switch(k.get("kind").getAsInt()){case 0->new A(o,i,h);case 1->new B(o,i,h);case 2->new C(o,i,h);case 3->new D(o,i,h);default->throw new AssertionError();};keys.add(key);identities.add(System.identityHashCode(key));}f.add("identities",identities);enriched.println(f);
var map=new HashMap<Key,Integer>();var saved=new HashMap<Key,Integer>();Iterator<Map.Entry<Key,Integer>> cursor=map.entrySet().iterator();for(var ov:f.getAsJsonArray("ops")){var op=ov.getAsJsonArray();int k=op.get(1).getAsInt();try{switch(op.get(0).getAsString()){
 case "put":out.println("old "+opt(map.put(keys.get(k),op.get(2).getAsInt())));break;
 case "remove":out.println("old "+opt(map.remove(keys.get(k))));break;
 case "get":out.println("get "+opt(map.get(keys.get(k))));break;
 case "copy":map=new HashMap<>(map);cursor=map.entrySet().iterator();out.println("copy");break;
 case "save":saved=new HashMap<>(map);out.println("save");break;
 case "swap":{var tmp=map;map=saved;saved=tmp;cursor=map.entrySet().iterator();out.println("swap");break;}
 case "clear":map.clear();out.println("clear");break;
 case "cursor":cursor=map.entrySet().iterator();out.println("cursor");break;
 case "next":if(cursor.hasNext()){var e=cursor.next();out.println("next "+e.getKey().object+"="+e.getValue());}else out.println("next none");break;
 case "force-next":{var e=cursor.next();out.println("next "+e.getKey().object+"="+e.getValue());break;}
 case "delete":cursor.remove();out.println("delete");break;
 default:throw new AssertionError();}}catch(ConcurrentModificationException e){out.println("error modified");}catch(IllegalStateException e){out.println("error current");}catch(NoSuchElementException e){out.println("next none");}
 for(int mi=0;mi<2;mi++){var current=mi==0?map:saved;var entries=new StringJoiner(",");for(var e:current.entrySet())entries.add(e.getKey().object+"="+e.getValue());out.println((mi==0?"map":"saved")+" "+current.size()+" "+current.isEmpty()+" "+cursor.hasNext()+" | "+entries);}

 }}}}
}
