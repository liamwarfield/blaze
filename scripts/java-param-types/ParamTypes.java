// Prints, for every @StarlarkMethod of Bazel's server jar, Java's parameter type error text
// (net/starlark/java/eval/ParamDescriptor.java:123 getTypeErrorMessage, with the class names of
// net/starlark/java/eval/Starlark.java:337 classType), computed by Bazel's own classes.
//
// Output (TSV, sorted): receiver<TAB>method<TAB>param<TAB>want
//   receiver: StarlarkAnnotations.getStarlarkBuiltin(declaring class).name(), or "" (global
//             methods, @GlobalMethods classes)
//   want:     the text after "want '" in "in call to M(), parameter 'P' got value of type 'T',
//             want '...'"; "-" when the parameter accepts Object (no check).
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.Enumeration;
import java.util.List;
import java.util.TreeSet;
import java.util.jar.JarEntry;
import java.util.jar.JarFile;
import net.starlark.java.annot.Param;
import net.starlark.java.annot.ParamType;
import net.starlark.java.annot.StarlarkAnnotations;
import net.starlark.java.annot.StarlarkBuiltin;
import net.starlark.java.annot.StarlarkMethod;
import net.starlark.java.eval.Starlark;

public class ParamTypes {
  public static void main(String[] args) throws Exception {
    TreeSet<String> out = new TreeSet<>();
    ClassLoader loader = ParamTypes.class.getClassLoader();
    try (JarFile jar = new JarFile(args[0])) {
      Enumeration<JarEntry> e = jar.entries();
      while (e.hasMoreElements()) {
        String n = e.nextElement().getName();
        if (!n.endsWith(".class")
            || !(n.startsWith("com/google/devtools/build/") || n.startsWith("net/starlark/java/"))) {
          continue;
        }
        String cn = n.substring(0, n.length() - 6).replace('/', '.');
        Class<?> c;
        Method[] methods;
        try {
          c = Class.forName(cn, false, loader);
          methods = c.getDeclaredMethods();
        } catch (Throwable t) {
          continue;
        }
        for (Method m : methods) {
          StarlarkMethod sm = m.getAnnotation(StarlarkMethod.class);
          if (sm == null) {
            continue;
          }
          StarlarkBuiltin b = StarlarkAnnotations.getStarlarkBuiltin(c);
          String receiver = b == null ? "" : b.name();
          Class<?>[] pts = m.getParameterTypes();
          Param[] params = sm.parameters();
          for (int i = 0; i < params.length; i++) {
            Param p = params[i];
            List<Class<?>> allowed = new ArrayList<>();
            if (p.allowedTypes().length > 0) {
              for (ParamType pt : p.allowedTypes()) {
                allowed.add(pt.type());
              }
            } else if (i < pts.length) {
              Class<?> pc = pts[i];
              allowed.add(pc == Boolean.TYPE ? Boolean.class : pc);
            } else {
              continue;
            }
            String want;
            if (allowed.contains(Object.class)) {
              want = "-";
            } else {
              List<String> names = new ArrayList<>();
              for (Class<?> a : allowed) {
                String t = Starlark.classType(a);
                if (!t.equals("NativeComputedDefault")) {
                  names.add(t);
                }
              }
              StringBuilder buf = new StringBuilder();
              for (int j = 0, k = names.size(); j < k; j++) {
                if (j > 0) {
                  buf.append(k == 2 ? " or " : j < k - 1 ? ", " : ", or ");
                }
                buf.append(names.get(j));
              }
              want = buf.toString();
            }
            out.add(receiver + "\t" + sm.name() + "\t" + p.name() + "\t" + want);
          }
        }
      }
    }
    for (String s : out) {
      System.out.println(s);
    }
  }
}
