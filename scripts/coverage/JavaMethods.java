// Reference parser for scripts/coverage/java-map check-parser: lists the methods and
// constructors of each Java file with javac's own parser (parse only, no symbol resolution).
// Run with a JDK >= 21 that has the jdk.compiler module:
//   java scripts/coverage/JavaMethods.java <root> <relpath>...      (or relpaths on stdin)
// Output, one line per method: relpath \t classpath \t name \t start_line \t end_line \t kind
// start_line is the first line of the modifiers (annotations included), as in java-map.
// Members of anonymous and local classes, enum-constant bodies and lambdas are not listed.
import com.sun.source.tree.*;
import com.sun.source.util.*;
import javax.tools.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;

public class JavaMethods {
  public static void main(String[] args) throws Exception {
    if (args.length < 1 || args[0].equals("help")) {
      System.out.println("usage: java JavaMethods.java <root> [relpath...]  (relpaths on stdin if none)");
      return;
    }
    Path root = Paths.get(args[0]);
    List<String> rels = new ArrayList<>();
    for (int i = 1; i < args.length; i++) rels.add(args[i]);
    if (rels.isEmpty()) {
      BufferedReader r = new BufferedReader(new InputStreamReader(System.in));
      for (String l; (l = r.readLine()) != null; ) if (!l.isBlank()) rels.add(l.trim());
    }
    JavaCompiler jc = ToolProvider.getSystemJavaCompiler();
    StandardJavaFileManager fm = jc.getStandardFileManager(null, null, null);
    PrintStream out = new PrintStream(new BufferedOutputStream(System.out), false, "UTF-8");
    for (String rel : rels) {
      JavacTask task = (JavacTask) jc.getTask(null, fm, d -> {}, List.of("-proc:none"), null,
          fm.getJavaFileObjects(root.resolve(rel).toFile()));
      SourcePositions sp = Trees.instance(task).getSourcePositions();
      for (CompilationUnitTree cu : task.parse()) {
        LineMap lm = cu.getLineMap();
        Deque<String> stack = new ArrayDeque<>();
        new TreeScanner<Void, Void>() {
          @Override public Void visitClass(ClassTree t, Void v) {
            stack.addLast(t.getSimpleName().toString());
            for (Tree m : t.getMembers()) {
              if (m instanceof ClassTree) scan(m, null);
              else if (m instanceof MethodTree mt) {
                long s = sp.getStartPosition(cu, mt), e = sp.getEndPosition(cu, mt);
                String name = mt.getName().toString();
                String kind = name.equals("<init>") ? "ctor" : (mt.getBody() == null ? "abstract" : "method");
                if (name.equals("<init>")) name = stack.peekLast();
                out.println(rel + "\t" + String.join(".", stack) + "\t" + name + "\t"
                    + lm.getLineNumber(s) + "\t" + lm.getLineNumber(e) + "\t" + kind);
              }
            }
            stack.removeLast();
            return null;
          }
        }.scan(cu, null);
      }
    }
    out.flush();
  }
}
