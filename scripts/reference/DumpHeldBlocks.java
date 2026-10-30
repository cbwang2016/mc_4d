import java.lang.reflect.*;
/** Uses ItemRenderer qa's actual dispatch methods, not similarly named block methods. */
public class DumpHeldBlocks {
 public static void main(String[] args)throws Exception {
  Class<?> blocks=Class.forName("to");Object[] all=(Object[])blocks.getField("m").get(null);
  boolean first=true;System.out.print("[");
  for(int id=1;id<all.length;id++){Object block=all[id];if(block==null)continue;
   if(!first)System.out.print(",");first=false;
   int render=(Integer)blocks.getMethod("g").invoke(block);
   boolean solid=render==0||render==13||render==10||render==11;
   blocks.getMethod("f").invoke(block);
   System.out.print("{\"reference_id\":"+id+",\"render_type\":"+render+",\"held_3d\":"+solid+",\"bounds\":[");
   for(int j=0;j<6;j++){if(j>0)System.out.print(",");System.out.print(blocks.getField("b"+(char)('q'+j)).get(block));}
   System.out.print("],\"tiles\":[");
   for(int m=0;m<16;m++){if(m>0)System.out.print(",");System.out.print("[");for(int side=0;side<6;side++){if(side>0)System.out.print(",");System.out.print(blocks.getMethod("a",int.class,int.class).invoke(block,side,m));}System.out.print("]");}
   System.out.print("]}");
  }System.out.println("]");
 }
}
