// Runs a Starlark file on Bazel 9.3.2's own interpreter (net.starlark.java from the oracle's
// A-server.jar) the way Bazel's ScriptTest does, and prints what the file prints. Used to record
// the Java side of crates/starlark_conformance/tests/data/*.star (see tests/java_recorded.rs).
//
// usage: java -cp <install_base>/A-server.jar StarlarkProbe.java latin1|unicode FILE
//   latin1:  ParserInput.fromLatin1(bytes), -internal_bazel_only_utf_8_byte_strings on
//            (Bazel's ScriptTest_Latin1 target; how Bazel reads BUILD and .bzl files)
//   unicode: ParserInput.fromUTF8(bytes), the flag off (Bazel's ScriptTest target)
// Predeclared: json, _utf8_byte_strings. Output is ASCII: each printed line with `\` as `\\`,
// chars below 0x20 and 0x7F..0xFF as \xNN, chars above 0xFF as backslash-u{NNNN} (hex). Syntax errors
// print `SYNTAX <location>: <message>` (every error of the file, in Java's order), evaluation
// errors `EVAL <message>` followed by one `TRACE <line>` per line of
// EvalException.getMessageWithStack (the traceback Bazel prints; source lines read as Latin-1,
// as Bazel's BlazeRuntime source reader does, BlazeRuntime.java:1452).
// <install_base> is ~/.cache/blaze/oracle-out/ur/install/58cf1ecbff441bac762b0050d6da21e9.

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.HashMap;
import java.util.Map;
import net.starlark.java.eval.EvalException;
import net.starlark.java.eval.Mutability;
import net.starlark.java.eval.Starlark;
import net.starlark.java.eval.StarlarkSemantics;
import net.starlark.java.eval.StarlarkThread;
import net.starlark.java.lib.json.Json;
import net.starlark.java.syntax.FileOptions;
import net.starlark.java.syntax.ParserInput;
import net.starlark.java.syntax.SyntaxError;

public class StarlarkProbe {
  static void out(String s) {
    StringBuilder b = new StringBuilder();
    for (char c : s.toCharArray()) {
      if (c == '\\') {
        b.append("\\\\");
      } else if (c < 0x20 || (c >= 0x7f && c <= 0xff)) {
        b.append(String.format("\\x%02x", (int) c));
      } else if (c > 0xff) {
        b.append(String.format("\\u{%04x}", (int) c));
      } else {
        b.append(c);
      }
    }
    System.out.println(b);
  }

  @SuppressWarnings("deprecation") // ParserInput.fromLatin1
  public static void main(String[] args) throws Exception {
    boolean latin1 = args[0].equals("latin1");
    byte[] bytes = Files.readAllBytes(Paths.get(args[1]));
    ParserInput input =
        latin1 ? ParserInput.fromLatin1(bytes, args[1]) : ParserInput.fromUTF8(bytes, args[1]);
    StarlarkSemantics semantics =
        StarlarkSemantics.builder()
            .setBool(StarlarkSemantics.INTERNAL_BAZEL_ONLY_UTF_8_BYTE_STRINGS, latin1)
            .build();
    Map<String, Object> predeclared = new HashMap<>();
    predeclared.put("json", Json.INSTANCE);
    predeclared.put("_utf8_byte_strings", latin1);
    try (Mutability mu = Mutability.create("probe")) {
      StarlarkThread thread = StarlarkThread.createTransient(mu, semantics);
      thread.setPrintHandler((th, msg) -> out(msg));
      net.starlark.java.eval.Module module =
          net.starlark.java.eval.Module.withPredeclared(semantics, predeclared);
      try {
        Starlark.execFile(input, FileOptions.DEFAULT, module, thread);
      } catch (SyntaxError.Exception e) {
        for (SyntaxError err : e.errors()) {
          out("SYNTAX " + err);
        }
      } catch (EvalException e) {
        out("EVAL " + e.getMessage());
        String stack =
            e.getMessageWithStack(
                loc -> {
                  try {
                    String content =
                        new String(
                            Files.readAllBytes(Paths.get(loc.file())),
                            StandardCharsets.ISO_8859_1);
                    String[] lines = content.split("\n", -1);
                    return loc.line() >= 1 && loc.line() <= lines.length
                        ? lines[loc.line() - 1]
                        : null;
                  } catch (Throwable unused) {
                    return null;
                  }
                });
        for (String line : stack.split("\n", -1)) {
          out("TRACE " + line);
        }
      }
    }
  }
}
