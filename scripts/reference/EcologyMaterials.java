// Reflection helper for the verified Minecraft Beta1.5 client.jar.
import java.lang.reflect.*;
class EcologyMaterials {public static void main(String[]args)throws Exception {Class<?> b=Class.forName("to"),m=Class.forName("kr");Object[] blocks=(Object[])b.getField("m").get(null);for(int id=0;id<blocks.length;id++)if(blocks[id]!=null){Object mat=b.getField("by").get(blocks[id]);System.out.println("MATERIAL "+id+" "+m.getMethod("c").invoke(mat)+" "+m.getMethod("e").invoke(mat));}}}
