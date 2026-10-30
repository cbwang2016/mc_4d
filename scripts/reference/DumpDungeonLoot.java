// Reflection helper for the verified Minecraft Beta1.5 client.jar.
public class DumpDungeonLoot { public static void main(String[] args)throws Exception {Class.forName("to");Class<?> c=Class.forName("gb");for(String name:new String[]{"ay","m","S","R","K","I","au","ar","aA","bb","aU"}){Object item=c.getField(name).get(null);System.out.println(name+"="+c.getField("bd").getInt(item));}}}
