// W5R debugging probe: runs pinned Bazel 9.3.2 classes from the oracle's A-server.jar through
// reflection (so it compiles with any javac >= 11). usage: see `run help`.
public class Probe {
  static String hex(byte[] b) {
    StringBuilder sb = new StringBuilder();
    for (byte x : b) sb.append(String.format("%02x", x));
    return sb.toString();
  }

  public static void main(String[] args) throws Exception {
    if (args.length == 0 || args[0].equals("help")) {
      System.out.println("usage: run semantics-fingerprint");
      return;
    }
    if (args[0].equals("semantics-fingerprint")) {
      Class<?> blo =
          Class.forName("com.google.devtools.build.lib.packages.semantics.BuildLanguageOptions");
      Object defaults =
          Class.forName("com.google.devtools.common.options.Options")
              .getMethod("getDefaults", Class.class)
              .invoke(null, blo);
      Object sem = blo.getMethod("toStarlarkSemantics").invoke(defaults);
      Object bs =
          blo.getMethod("stableFingerprint", Class.forName("net.starlark.java.eval.StarlarkSemantics"))
              .invoke(null, sem);
      byte[] b = (byte[]) bs.getClass().getMethod("toByteArray").invoke(bs);
      System.out.println(hex(b));
    }
    if (args[0].equals("attr-json")) {
      // SINGLE_EXTENSION_USAGES_VALUE_GSON.toJson(AttributeValues.create(dict)) for sample values.
      ClassLoader cl = Probe.class.getClassLoader();
      Class<?> starlark = Class.forName("net.starlark.java.eval.Starlark");
      Class<?> dictC = Class.forName("net.starlark.java.eval.Dict");
      Class<?> listC = Class.forName("net.starlark.java.eval.StarlarkList");
      Class<?> intC = Class.forName("net.starlark.java.eval.StarlarkInt");
      Class<?> labelC = Class.forName("com.google.devtools.build.lib.cmdline.Label");
      java.util.LinkedHashMap<String, Object> m = new java.util.LinkedHashMap<>();
      m.put("s", "str");
      m.put("b", true);
      m.put("i", intC.getMethod("of", int.class).invoke(null, 42));
      m.put("l", listC.getMethod("immutableCopyOf", Iterable.class).invoke(null, java.util.List.of("a", "b")));
      Object label = labelC.getMethod("parseCanonicalUnchecked", String.class).invoke(null, "@@foo+//bar:baz");
      m.put("lab", label);
      m.put("labs", listC.getMethod("immutableCopyOf", Iterable.class).invoke(null, java.util.List.of(label)));
      m.put("n", starlark.getField("NONE").get(null));
      java.util.LinkedHashMap<String, Object> d = new java.util.LinkedHashMap<>();
      d.put("k", "v");
      m.put("d", dictC.getMethod("immutableCopyOf", java.util.Map.class).invoke(null, d));
      m.put("esc", "@@x");
      Object dict = dictC.getMethod("immutableCopyOf", java.util.Map.class).invoke(null, m);
      Class<?> av = Class.forName("com.google.devtools.build.lib.bazel.bzlmod.AttributeValues");
      Object values = av.getMethod("create", dictC).invoke(null, dict);
      Class<?> g = Class.forName("com.google.devtools.build.lib.bazel.bzlmod.GsonTypeAdapterUtil");
      Object gson = g.getField("SINGLE_EXTENSION_USAGES_VALUE_GSON").get(null);
      System.out.println(gson.getClass().getMethod("toJson", Object.class).invoke(gson, values));
    }
  }
}
