// Records how protobuf-java (as linked into Bazel 9.3.2's A-server.jar) decodes malformed UTF-8 in
// proto `string` values, as test vectors for crates/blaze_protos. See `run help`.
import build.bazel.remote.execution.v2.OutputFile;
import com.google.devtools.build.lib.query2.proto.proto2api.Build;
import com.google.protobuf.ByteString;
import com.google.protobuf.DynamicMessage;
import com.google.protobuf.TextFormat;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;

public class ProtobufUtf8Vectors {
  static String hex(byte[] b) {
    StringBuilder sb = new StringBuilder();
    for (byte x : b) sb.append(String.format("%02x", x & 0xff));
    return sb.length() == 0 ? "-" : sb.toString();
  }

  static byte[] unhex(String s) {
    if (s.equals("-")) return new byte[0];
    byte[] b = new byte[s.length() / 2];
    for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(s.substring(2 * i, 2 * i + 2), 16);
    return b;
  }

  static String octal(byte[] b) {
    StringBuilder sb = new StringBuilder();
    for (byte x : b) sb.append(String.format("\\%03o", x & 0xff));
    return sb.toString();
  }

  static byte[] utf8(String s) {
    return s.getBytes(StandardCharsets.UTF_8);
  }

  static byte[] field1(byte[] b) {
    // tag 1, wire type 2; every input is shorter than 128 bytes.
    byte[] out = new byte[b.length + 2];
    out[0] = 0x0a;
    out[1] = (byte) b.length;
    System.arraycopy(b, 0, out, 2, b.length);
    return out;
  }

  public static void main(String[] args) throws Exception {
    Method unescapeText = TextFormat.class.getDeclaredMethod("unescapeText", String.class);
    unescapeText.setAccessible(true);
    BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
    System.out.println("# input-hex kind result-hex (or ERR:message, spaces as _)");
    String line;
    while ((line = in.readLine()) != null) {
      line = line.trim();
      if (line.isEmpty() || line.startsWith("#")) continue;
      byte[] b = unhex(line);
      // TextFormat parser: Tokenizer.consumeString on a proto3 string field.
      OutputFile.Builder ob = OutputFile.newBuilder();
      TextFormat.merge("path: \"" + octal(b) + "\"", ob);
      OutputFile of = ob.build();
      System.out.println(line + " parse " + hex(utf8(of.getPath())));
      // TextFormat printer, both escaping modes, of the parsed message.
      System.out.println(line + " print " + hex(utf8(TextFormat.printer().printToString(of))));
      System.out.println(
          line + " print_raw "
              + hex(utf8(TextFormat.printer().escapingNonAscii(false).printToString(of))));
      // TextFormat.unescapeText.
      System.out.println(line + " unescape_text " + hex(utf8((String) unescapeText.invoke(null, octal(b)))));
      // DynamicMessage, proto2 string field (CodedInputStream.readString).
      DynamicMessage d2 =
          DynamicMessage.parseFrom(Build.License.getDescriptor(), field1(b));
      String v2 = (String) d2.getRepeatedField(Build.License.getDescriptor().findFieldByNumber(1), 0);
      System.out.println(line + " dynamic_proto2 " + hex(utf8(v2)));
      // DynamicMessage, proto3 string field (readStringRequireUtf8).
      String r3;
      try {
        DynamicMessage d3 = DynamicMessage.parseFrom(OutputFile.getDescriptor(), field1(b));
        r3 = hex(utf8((String) d3.getField(OutputFile.getDescriptor().findFieldByNumber(1))));
      } catch (Exception e) {
        r3 = "ERR:" + e.getMessage().replace(' ', '_');
      }
      System.out.println(line + " dynamic_proto3 " + r3);
    }
  }
}
