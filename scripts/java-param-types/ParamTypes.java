// Prints, for every Starlark method of every class in Bazel's server jar, Java's parameter type
// error text, by calling Bazel's own code: net/starlark/java/eval/MethodDescriptor.java
// MethodDescriptor.of (which builds each ParamDescriptor with ParamDescriptor.of) and
// ParamDescriptor.getTypeErrorMessage / getAllowedClasses, reached by reflection (they are
// package-private). Methods are found as CallUtils.buildStarlarkClassDescriptor finds them:
// Class.getMethods() (inherited ones included), non-synthetic, with
// StarlarkAnnotations.getStarlarkMethod.
//
// Output (TSV, sorted, distinct): receiver<TAB>method<TAB>param<TAB>want
//   receiver: StarlarkAnnotations.getStarlarkBuiltin(class).name() of the class the method is
//             called on (so a method inherited by a builtin class is listed under that class),
//             or "" for a class without one (global functions: MethodLibrary, @GlobalMethods
//             classes, abstract bases)
//   want:     ParamDescriptor.getTypeErrorMessage(); "-" when getAllowedClasses() is null (the
//             parameter accepts Object: no check)
import java.lang.reflect.Method;
import java.util.Arrays;
import java.util.Comparator;
import java.util.Enumeration;
import java.util.List;
import java.util.TreeSet;
import java.util.jar.JarEntry;
import java.util.jar.JarFile;
import net.starlark.java.annot.StarlarkAnnotations;
import net.starlark.java.annot.StarlarkBuiltin;
import net.starlark.java.annot.StarlarkMethod;

public class ParamTypes {
  public static void main(String[] args) throws Exception {
    ClassLoader loader = ParamTypes.class.getClassLoader();
    Class<?> methodDescriptor = Class.forName("net.starlark.java.eval.MethodDescriptor");
    Method of = methodDescriptor.getDeclaredMethod("of", Method.class, StarlarkMethod.class);
    of.setAccessible(true);
    Method getParameters = methodDescriptor.getDeclaredMethod("getParameters");
    getParameters.setAccessible(true);
    Class<?> paramDescriptor = Class.forName("net.starlark.java.eval.ParamDescriptor");
    Method getName = paramDescriptor.getDeclaredMethod("getName");
    getName.setAccessible(true);
    Method getAllowedClasses = paramDescriptor.getDeclaredMethod("getAllowedClasses");
    getAllowedClasses.setAccessible(true);
    Method getTypeErrorMessage = paramDescriptor.getDeclaredMethod("getTypeErrorMessage");
    getTypeErrorMessage.setAccessible(true);

    TreeSet<String> out = new TreeSet<>();
    int failures = 0;
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
          methods = c.getMethods();
        } catch (Throwable t) {
          continue;
        }
        Arrays.sort(methods, Comparator.comparing(Method::getName));
        StarlarkBuiltin b;
        try {
          b = StarlarkAnnotations.getStarlarkBuiltin(c);
        } catch (Throwable t) {
          continue;
        }
        String receiver = b == null ? "" : b.name();
        for (Method m : methods) {
          if (m.isSynthetic()) {
            continue;
          }
          StarlarkMethod sm;
          try {
            sm = StarlarkAnnotations.getStarlarkMethod(m);
          } catch (Throwable t) {
            continue;
          }
          if (sm == null) {
            continue;
          }
          Object[] params;
          try {
            params = (Object[]) getParameters.invoke(of.invoke(null, m, sm));
          } catch (Throwable t) {
            failures++;
            System.err.println("MethodDescriptor.of failed: " + cn + "." + m.getName() + ": " + t);
            continue;
          }
          for (Object p : params) {
            String want =
                getAllowedClasses.invoke(p) == null ? "-" : (String) getTypeErrorMessage.invoke(p);
            out.add(receiver + "\t" + sm.name() + "\t" + getName.invoke(p) + "\t" + want);
          }
        }
      }
    }
    for (String s : out) {
      System.out.println(s);
    }
    System.err.println("rows: " + out.size() + ", failures: " + failures);
  }
}
