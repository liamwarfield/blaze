// Prints the boundaries of java.text.BreakIterator.getLineInstance() for each input line.
// Used by scripts/cmds/java-line-breaks; see there.
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.text.BreakIterator;
import java.util.StringJoiner;

public class LineBreaks {
  static String unescape(String s) {
    StringBuilder out = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      char c = s.charAt(i);
      if (c != '\\' || i + 1 >= s.length()) { out.append(c); continue; }
      char n = s.charAt(++i);
      switch (n) {
        case 't' -> out.append('\t');
        case 'r' -> out.append('\r');
        case 'n' -> out.append('\n');
        case '\\' -> out.append('\\');
        case 'u' -> { out.append((char) Integer.parseInt(s.substring(i + 1, i + 5), 16)); i += 4; }
        default -> throw new IllegalArgumentException("bad escape \\" + n);
      }
    }
    return out.toString();
  }

  public static void main(String[] args) throws Exception {
    BufferedReader in =
        new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
    String line;
    while ((line = in.readLine()) != null) {
      if (line.isEmpty() || line.startsWith("#")) continue;
      String text = unescape(line);
      BreakIterator b = BreakIterator.getLineInstance();
      b.setText(text);
      StringJoiner j = new StringJoiner(",");
      for (int p = b.first(); p != BreakIterator.DONE; p = b.next()) j.add(Integer.toString(p));
      System.out.println(j + "\t" + line);
    }
  }
}
