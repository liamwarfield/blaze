import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;

/**
 * Records how the JDK's `new String(bytes, UTF_8)` replaces malformed input, for
 * crates/blaze_util/tests/string_encoding_test.rs (see scripts/string-encoding/run).
 *
 * Output lines:
 *   V <hex input> <hex UTF-8 of the decoded string>     a named sample vector
 *   D <alphabet hex> <max length> <count> <sha256>      digest over every sequence of 1..max
 *                                                       bytes of the alphabet, in odometer order
 *                                                       (last byte fastest); each sequence adds
 *                                                       be32(len(out)) + out to the digest
 */
public class Utf8DecodeVectors {
  static final int[] WIDE = {
    0x41, 0x7f, 0x80, 0x8f, 0x90, 0x9f, 0xa0, 0xbf, 0xc0, 0xc1, 0xc2, 0xdf, 0xe0, 0xe1, 0xec,
    0xed, 0xee, 0xef, 0xf0, 0xf1, 0xf3, 0xf4, 0xf5, 0xf7, 0xf8, 0xff
  };
  static final int[] NARROW = {0x41, 0x80, 0x90, 0xa0, 0xbf, 0xe0, 0xed, 0xf0, 0xf4, 0xff};

  static final String[] SAMPLES = {
    "c3a9", "ff", "fffe01", "c3", "c341", "c0af", "c1bf", "e0", "e080", "e08080", "e0a0",
    "e0a041", "e2", "e282", "e28241", "e282ac", "eda080", "edb080", "eda0", "ed9fbf", "f0",
    "f09f", "f09f90", "f09f90b7", "f09f9041", "f09f41", "f041", "f0808080", "f08f8080",
    "f4908080", "f48fbfbf", "f5808080", "f8888080", "80", "bf", "41ff42", "f09f9080",
    "f490", "f09041", "e1808041", "efbfbd", "007f80feff"
  };

  static String hex(byte[] b) {
    StringBuilder sb = new StringBuilder();
    for (byte x : b) {
      sb.append(String.format("%02x", x & 0xff));
    }
    return sb.toString();
  }

  static byte[] unhex(String s) {
    byte[] b = new byte[s.length() / 2];
    for (int i = 0; i < b.length; i++) {
      b[i] = (byte) Integer.parseInt(s.substring(2 * i, 2 * i + 2), 16);
    }
    return b;
  }

  static byte[] decode(byte[] in) {
    return new String(in, StandardCharsets.UTF_8).getBytes(StandardCharsets.UTF_8);
  }

  static void digest(int[] alphabet, int maxLen) throws Exception {
    MessageDigest md = MessageDigest.getInstance("SHA-256");
    long count = 0;
    for (int len = 1; len <= maxLen; len++) {
      int[] idx = new int[len];
      byte[] seq = new byte[len];
      while (true) {
        for (int i = 0; i < len; i++) {
          seq[i] = (byte) alphabet[idx[i]];
        }
        byte[] out = decode(seq);
        md.update(new byte[] {
          (byte) (out.length >>> 24), (byte) (out.length >>> 16), (byte) (out.length >>> 8),
          (byte) out.length
        });
        md.update(out);
        count++;
        int p = len - 1;
        while (p >= 0 && ++idx[p] == alphabet.length) {
          idx[p] = 0;
          p--;
        }
        if (p < 0) {
          break;
        }
      }
    }
    byte[] a = new byte[alphabet.length];
    for (int i = 0; i < a.length; i++) {
      a[i] = (byte) alphabet[i];
    }
    System.out.println("D " + hex(a) + " " + maxLen + " " + count + " " + hex(md.digest()));
  }

  public static void main(String[] args) throws Exception {
    System.out.println("# JDK " + System.getProperty("java.version"));
    for (String s : SAMPLES) {
      System.out.println("V " + s + " " + hex(decode(unhex(s))));
    }
    digest(WIDE, 4);
    digest(NARROW, 6);
  }
}
