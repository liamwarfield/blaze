// Records what Bazel 9.3.2's own classes (A-server.jar) return for converter inputs, as test
// vectors for crates/blaze_options and crates/blaze_optdefs. See `run help`.
import com.github.benmanes.caffeine.cache.CaffeineSpec;
import com.google.common.primitives.Doubles;
import com.google.devtools.common.options.Converters;
import com.google.devtools.common.options.OptionsParsingException;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.time.Duration;

public class OptionsConverterVectors {
  static String field(Object o, String name) throws Exception {
    Field f = CaffeineSpec.class.getDeclaredField(name);
    f.setAccessible(true);
    Object v = f.get(o);
    if (v instanceof Duration) {
      Duration d = (Duration) v;
      return d.getSeconds() + "s" + d.getNano() + "n";
    }
    return String.valueOf(v);
  }

  public static void main(String[] args) throws Exception {
    BufferedReader in =
        new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
    String line;
    while ((line = in.readLine()) != null) {
      if (line.isEmpty() || line.startsWith("#")) continue;
      int tab = line.indexOf('\t');
      String kind = line.substring(0, tab);
      String input = line.substring(tab + 1);
      String result;
      switch (kind) {
        case "caffeine":
          try {
            CaffeineSpec spec = new Converters.CaffeineSpecConverter().convert(input);
            StringBuilder sb = new StringBuilder("OK:").append(spec);
            for (String f :
                new String[] {
                  "initialCapacity", "maximumSize", "maximumWeight", "keyStrength",
                  "valueStrength", "expireAfterWrite", "expireAfterAccess", "refreshAfterWrite",
                  "recordStats"
                }) {
              sb.append('|').append(field(spec, f));
            }
            result = sb.toString();
          } catch (OptionsParsingException e) {
            result = "ERR:" + e.getMessage();
          } catch (RuntimeException e) {
            result = "EXC:" + e.getClass().getName();
          }
          break;
        case "double":
          Double d = Doubles.tryParse(input);
          result = d == null ? "null" : Long.toHexString(Double.doubleToRawLongBits(d));
          break;
        default:
          throw new IllegalArgumentException(kind);
      }
      System.out.println(kind + "\t" + input + "\t" + result);
    }
  }
}
