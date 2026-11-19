// Expected orders for platform_utils_test.rs `*_keep_java_map_order`: PlatformUtils.getPlatformProto
// (Bazel 9.3.2, lib/analysis/platform/PlatformUtils.java:102-177) with the same map types, applied
// to internal property names that are invalid UTF-8 (all become "x�" after
// internalToUnicode), then the stable sort by name. Prints the values in the resulting order.
// Run: java crates/blaze_remote/tests/data/PlatformPropertyOrder.java
import java.nio.charset.StandardCharsets;
import java.util.*;

public class PlatformPropertyOrder {
  static String internalToUnicode(String s) {
    return new String(s.getBytes(StandardCharsets.ISO_8859_1), StandardCharsets.UTF_8);
  }

  static Map<String, String> linked(String... kv) {
    Map<String, String> m = new LinkedHashMap<>();
    for (int i = 0; i < kv.length; i += 2) m.put(kv[i], kv[i + 1]);
    return m;
  }

  static SortedMap<String, String> sorted(String... kv) {
    return new TreeMap<>(linked(kv));
  }

  static void run(
      String name,
      SortedMap<String, String> defaultExecProperties,
      Map<String, String> combined,
      boolean hasPlatform,
      Map<String, String> platformExecProperties,
      Map<String, String> additionalProperties) {
    Map<String, String> properties = new HashMap<>();
    if (!combined.isEmpty()) {
      if (!hasPlatform || platformExecProperties.isEmpty()) {
        properties.putAll(defaultExecProperties);
        properties.putAll(combined);
      } else {
        properties = combined;
      }
    } else if (hasPlatform) {
      properties.putAll(platformExecProperties);
    }
    if (properties.isEmpty()) {
      properties = defaultExecProperties;
    }
    if (!additionalProperties.isEmpty()) {
      if (properties.isEmpty()) {
        properties = additionalProperties;
      } else {
        properties = new HashMap<>(properties);
        properties.putAll(additionalProperties);
      }
    }
    List<String[]> list = new ArrayList<>();
    for (Map.Entry<String, String> e : properties.entrySet()) {
      list.add(new String[] {internalToUnicode(e.getKey()), internalToUnicode(e.getValue())});
    }
    list.sort(Comparator.comparing(p -> p[0]));
    StringBuilder sb = new StringBuilder(name + ":");
    for (String[] p : list) sb.append(' ').append(p[1]);
    System.out.println(sb);
  }

  public static void main(String[] a) {
    String k1 = "xÿ", k2 = "xþ", k3 = "x\u0080", k4 = "xÃ", k5 = "xé", k6 = "x¿";
    // Branch: combined exec properties, no execution platform: HashMap(defaults + combined).
    run("combined_without_platform", sorted(k1, "d1", k2, "d2"), linked(k3, "c3", k4, "c4", k5, "c5"),
        false, linked(), linked());
    // Branch: combined exec properties with a platform that has exec properties: ImmutableMap.
    run("combined_with_platform", sorted(), linked(k5, "c5", k3, "c3", k1, "c1"),
        true, linked("p", "1"), linked());
    // Branch: platform exec properties only: HashMap.putAll.
    run("platform_exec_properties", sorted(), linked(), true,
        linked(k6, "p6", k1, "p1", k4, "p4", k2, "p2"), linked());
    // Branch: default exec properties only: the SortedMap.
    run("defaults_only", sorted(k4, "d4", k1, "d1", k3, "d3"), linked(), false, linked(), linked());
    // Branch: additional properties only.
    run("additional_only", sorted(), linked(), false, linked(), linked(k2, "a2", k6, "a6", k1, "a1"));
    // Branch: merge into new HashMap<>(properties).
    run("merged_additional", sorted(k1, "d1", k3, "d3"), linked(), false, linked(),
        linked(k5, "a5", k2, "a2", k1, "a1x"));
  }
}
