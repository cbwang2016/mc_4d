import java.lang.reflect.*;import java.util.*;
public class DumpModels {
 static Object get(Object o,String name)throws Exception {Field f=o.getClass().getDeclaredField(name);f.setAccessible(true);return f.get(o);}
 static void part(String name,Object p)throws Exception {
  Object[] vertices=(Object[])get(p,"j");if(vertices==null)return;
  double[] lo={1e9,1e9,1e9},hi={-1e9,-1e9,-1e9};
  for(Object v:vertices){Object b=get(v,"a");for(int i=0;i<3;i++){double n=((Number)get(b,""+(char)('a'+i))).doubleValue();lo[i]=Math.min(lo[i],n);hi[i]=Math.max(hi[i],n);}}
  System.out.print("{\"name\":\""+name+"\",\"lo\":"+Arrays.toString(lo)+",\"hi\":"+Arrays.toString(hi)+",\"pose\":[");
  for(int i=0;i<6;i++){if(i>0)System.out.print(",");System.out.print(get(p,""+(char)('a'+i)));}
  Object[] faces=(Object[])get(p,"k");double[] ext=new double[3];
  for(int face:new int[]{0,2}){Object[] vv=(Object[])get(faces[face],"a");double u0=1e9,u1=-1e9,v0=1e9,v1=-1e9;for(Object v:vv){double u=((Number)get(v,"b")).doubleValue(),t=((Number)get(v,"c")).doubleValue();u0=Math.min(u0,u);u1=Math.max(u1,u);v0=Math.min(v0,t);v1=Math.max(v1,t);}if(face==0){ext[2]=(u1-u0)*64;ext[1]=(v1-v0)*32;}else{ext[0]=(u1-u0)*64;}}
  System.out.print("],\"uv\":["+get(p,"l")+","+get(p,"m")+"],\"size\":"+Arrays.toString(ext)+"}");
 }
 public static void main(String[]args)throws Exception {
  String[] names={"dx","gv","ma","dr","rz","vf","dz","kg","qz","hw","ms","n","n_sitting","ep","mu"};System.out.print("[");boolean firstModel=true;
  for(String name:names){Class<?> c=Class.forName(name.equals("n_sitting")?"n":name);Object model=name.equals("ms")?c.getConstructor(int.class).newInstance(0):c.getConstructor().newInstance();
   if(name.startsWith("n")){Object wolf=Class.forName("fx").getConstructor(Class.forName("et")).newInstance(new Object[]{null});if(name.equals("n_sitting"))wolf.getClass().getMethod("b",boolean.class).invoke(wolf,true);c.getMethod("a",Class.forName("kw"),float.class,float.class,float.class).invoke(model,wolf,0f,0f,0f);}
   try{c.getMethod("b",float.class,float.class,float.class,float.class,float.class,float.class).invoke(model,0f,0f,0f,0f,0f,.0625f);}catch(NoSuchMethodException e){}
   if(!firstModel)System.out.print(",");firstModel=false;System.out.print("{\"class\":\""+name+"\",\"parts\":[");boolean first=true;
   java.util.List<Field> fields=new java.util.ArrayList<Field>();for(Class<?> cc=c;cc!=null;cc=cc.getSuperclass())for(Field ff:cc.getDeclaredFields()){ff.setAccessible(true);fields.add(ff);}
   for(Field f:fields) {if(f.getType().getName().equals("ot")){Object p=f.get(model);if(p==null)continue;if(!first)System.out.print(",");first=false;part(f.getName(),p);}else if(f.getType().getName().equals("[Lot;")){Object[] ps=(Object[])f.get(model);for(int i=0;i<ps.length;i++){if(ps[i]==null)continue;if(!first)System.out.print(",");first=false;part(f.getName()+i,ps[i]);}}}
   System.out.print("]}");
  }System.out.println("]");
 }
}
