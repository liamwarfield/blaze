// Generates crates/blaze_util/src/java_regex/case_table.rs (JDK 25: Character.toUpperCase and
// toLowerCase of every code point either changes):
//   java crates/blaze_util/tests/data/JavaCaseTable.java
public class JavaCaseTable {
  public static void main(String[] a) {
    StringBuilder sb = new StringBuilder();
    int n = 0;
    for (int c = 0; c <= Character.MAX_CODE_POINT; c++) {
      int up = Character.toUpperCase(c), lo = Character.toLowerCase(c);
      if (up == c && lo == c) continue;
      sb.append(String.format("(0x%X, 0x%X, 0x%X), ", c, up, lo));
      if (++n % 4 == 0) sb.append("\n");
    }
    System.out.println("// " + n + " entries");
    System.out.println(sb);
  }
}
