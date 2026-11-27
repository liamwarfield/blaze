// Generates the rows of crates/blaze_util/tests/java_regex_test.rs from java.util.regex.Pattern.
//
// Run (JDK 25, the JDK embedded in Bazel 9.3.2):
//   java crates/blaze_util/tests/data/JavaRegexRows.java > rows.txt
// and paste the output between the ROWS markers of the test.
//
// Each row: (pattern, flags, input, expectation, loud). The expectation is
// Ok(find span in code points or None, matches()) or Err(PatternSyntaxException.getMessage()).
// `loud` marks a row whose construct Blaze does not port: Blaze must panic with
// "not ported yet" there (the Java result is kept for the record).
import java.nio.charset.StandardCharsets;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import java.util.regex.PatternSyntaxException;

public class JavaRegexRows {
  /** An internal string (StringEncoding.unicodeToInternal): one char per UTF-8 byte. */
  static String internal(String u) {
    return new String(u.getBytes(StandardCharsets.UTF_8), StandardCharsets.ISO_8859_1);
  }

  static String rs(String s) {
    StringBuilder b = new StringBuilder("\"");
    s.codePoints()
        .forEach(
            c -> {
              if (c == '\\') b.append("\\\\");
              else if (c == '"') b.append("\\\"");
              else if (c >= 0x20 && c < 0x7f) b.append((char) c);
              else b.append(String.format("\\u{%x}", c));
            });
    return b.append("\"").toString();
  }

  static void row(String p, int f, String in, boolean loud) {
    String expect;
    try {
      Pattern pat = Pattern.compile(p, f);
      Matcher m = pat.matcher(in);
      String find =
          m.find()
              ? "Some(("
                  + in.codePointCount(0, m.start())
                  + ", "
                  + in.codePointCount(0, m.end())
                  + "))"
              : "None";
      expect = "Ok((" + find + ", " + pat.matcher(in).matches() + "))";
    } catch (PatternSyntaxException e) {
      expect = "Err(" + rs(e.getMessage()) + ")";
    }
    System.out.println("    (" + rs(p) + ", " + f + ", " + rs(in) + ", " + expect + ", " + loud + "),");
  }

  static void t(String p, String in) {
    row(p, 0, in, false);
  }

  static void t(String p, int f, String in) {
    row(p, f, in, false);
  }

  static void loud(String p, String in) {
    row(p, 0, in, true);
  }

  static void loud(String p, int f, String in) {
    row(p, f, in, true);
  }

  public static void main(String[] a) {
    final int I = Pattern.CASE_INSENSITIVE, S = Pattern.DOTALL, M = Pattern.MULTILINE;
    final int L = Pattern.LITERAL, U = Pattern.UNICODE_CASE;
    // Predefined classes: ASCII in Java.
    t("\\w+", internal("é"));
    t("\\w+", "é");
    t("\\w+", internal("aéb"));
    t("\\w+", "a_9Z");
    t("\\W", internal("é"));
    t("\\W", "é");
    t("\\d", internal("²"));
    t("\\d", "٣");
    t("\\D+", "a٣");
    t("\\s", internal("a b"));
    t("\\s", "a b");
    t("\\s", "\u000b");
    t("\\S+", " é ");
    t("[\\w]+", internal("aéb"));
    t("[^\\w]", internal("é"));
    t("[\\w&&[^\\d]]", "5");
    t("[\\w&&[^\\d]]+", "a5b");
    // Word boundaries.
    t("\\bfoo\\b", internal("éfooé"));
    t("\\bfoo\\b", "éfooé");
    t("\\bfoo\\b", "a foo b");
    t("\\bé", " é");
    t("a\\b", "aé");
    t("\\B", "ab");
    t("a\\Bb", "ab");
    t("\\B", "a b");
    loud("a\\b", "áb");
    // Dot and line terminators.
    t(".*", internal("Å"));
    t("a.b", "a\rb");
    t("a.b", "a\nb");
    t("a..b", internal("a\u0085b"));
    t("a.b", "a\u0085b");
    t("a.b", "a b");
    t("a.b", "a b");
    t("(?s)a..b", internal("a\u0085b"));
    t("a.b", S, "a\nb");
    t("(?s:.)", "\n");
    t("(?s)a(?-s:.)b", "a\nb");
    t(".", "😀");
    t("^.$", "😀");
    // Case-insensitive: ASCII only.
    t("(?i)é", internal("É"));
    t("(?i)é", "É");
    t("é", I, "É");
    t("(?i)foo", "FOO");
    t("FOO", I, "foo");
    t("(?i)[a-c]+", "ABC");
    t("(?i)[A-C]+", "abc");
    t("(?i)[^a-z]", "Q");
    t("(?i)\\x41", "a");
    t("(?i)Ã", "ã");
    t("(?i)Ã", internal("é"));
    t("(?i)[À-Þ]", "é");
    t("(?i)[X-c]", "x");
    t("(?i)[X-c]", "C");
    t("(?i)[X-c]", "^");
    t("(?i:x)y", "XY");
    t("(?i:x)y", "Xy");
    t("a(?i)b|c", "C");
    t("(a(?i)b)c", "aBC");
    t("(a(?i)b)c", "aBc");
    t("(?i)a(?-i)b", "Ab");
    t("(?i)a(?-i)b", "AB");
    t("(?i)k", "K");
    t("(?i)\\p{Lower}", "A");
    t("(?i)\\p{Lu}", "a");
    t("\\p{Lu}", "a");
    t("(?iu)é", "É");
    t("é", I | U, "É");
    t("(?iu)ß", "ẞ");
    t("(?iu)ßa", "ẞa");
    t("(?iu)k", "\u212a");
    t("(?iu)[é]", "É");
    t("(?iu)[à-ä]", "Â");
    t("(?iu)[ÿ]", "Ÿ");
    t("(?iu)[µ]", "Μ");
    t("(?iu)ǅ", "ǆ");
    t("(?iu)σ", "ς");
    t("(?iu)ς", "Σ");
    t("(?iu)Σ", "ς");
    t("(?iu)[σ]", "ς");
    t("(?iu)i", "İ");
    t("(?iu)ı", "I");
    t("(?iu)[a-z]+", "\u212aİ");
    t("(?iu)[ǅ-ǆ]", "Ǆ");
    t("(?ui)äöüÄÖÜß🌱", "ÄÖÜäöüß🌱");
    t("(?iu)𐐀", "𐐨");
    t("(?U)(?i)é", "É");
    t("(?iu)é", internal("É"));
    t("(?u)é", "É");
    // Literals and escapes.
    t("é", internal("é"));
    t("é", "é");
    t("\\<", "<");
    t("\\>", ">");
    t("\\<a\\>", "<a>");
    t("\\-\\]\\}\\#", "-]}#");
    t("\\y", "y");
    t("\\E", "E");
    t("\\g", "g");
    t("\\0101", "A");
    t("\\0377", "ÿ");
    t("\\0400", " 0");
    t("\\07", "\u0007");
    t("\\08", "x");
    t("\\cA", "\u0001");
    t("\\c?", "\u007f");
    t("a\\c", "a");
    t("\\e", "\u001b");
    t("\\a", "\u0007");
    t("\\t\\n\\r\\f", "\t\n\r\f");
    t("\\x41", "A");
    t("\\xe9", "é");
    t("\\x{e9}", "é");
    t("\\x{1F600}", "😀");
    t("\\x{110000}", "x");
    t("\\x{12", "x");
    t("\\xg", "x");
    t("\\u00e9", "é");
    t("\\uD83D\\uDE00", "😀");
    t("\\u00g9", "x");
    t("a\\", "a");
    t("😀+", "😀😀");
    // \Q...\E.
    t("\\Qa.b\\E", "a.b");
    t("\\Qa.b\\E", "axb");
    t("\\Qab", "ab");
    t("\\Q(a\\E+", "(aa");
    t("[\\Qa]\\E]", "]");
    t("[\\Qa-c\\E]", "b");
    t("[\\Qa-c\\E]", "-");
    t("[x-\\Qz\\E]", "y");
    t("\\Q\\\\E", "\\");
    t("\\x\\Q41\\E", "x41");
    t("a|\\Qb|c", "b|c");
    // Predefined horizontal / vertical / line breaks.
    t("\\h", internal(" "));
    t("\\h", " ");
    t("\\H", "\t");
    t("\\v", internal("\u0085"));
    t("\\v", "\u000b");
    t("\\V", "a");
    t("[\\v-\\r]", "\f");
    t("\\R", "\r\n");
    t("\\R\\n", "\r\n");
    t("a\\R", "a ");
    t("[\\R]", "a");
    // Classes: Java's syntax.
    t("[]a]", "]");
    t("[^]a]", "b");
    t("[]", "a");
    t("[^]", "a");
    t("[^a[b]]", "b");
    t("[^a[b]]", "c");
    t("[a[b]]", "b");
    t("[a[b]&&[b]]", "a");
    t("[^a&&b]", "c");
    t("[a-c&&b]", "b");
    t("[a-c&&b]", "a");
    t("[a-z&&[^aeiou]]+", "xyz");
    t("[a-z&&[^aeiou]]+", "aei");
    t("[a&&]", "a");
    t("[&&a]", "a");
    t("[a&b]", "&");
    t("[a&&b&&c]", "b");
    t("[[:alpha:]]", ":");
    t("[[:alpha:]]", "x");
    t("[a-]", "-");
    t("[-a]", "-");
    t("[a-\\d]", "-");
    t("[\\d-a]", "-");
    t("[z-a]", "b");
    t("[a--b]", "-");
    t("[a~~b]", "~");
    t("[a-b-c]", "-");
    t("[a-[b]]", "-");
    t("[a-[b]]", "b");
    t("[\\b]", "\b");
    t("[\\A]", "A");
    t("[\\1]", "1");
    t("[ab", "a");
    t("[\\p{L}&&[^a]]", "b");
    t("[^\\p{L}]", "1");
    t("[\\x41-\\x43]", "B");
    t("[\\u00e0-\\u00ff]", "é");
    t("[é]", internal("é"));
    t("[^é]", internal("é"));
    t("[a^]", "^");
    t("[a&&^b]", "a");
    // Properties.
    t("\\p{Alpha}", "é");
    t("\\p{Alpha}", "e");
    t("\\p{Alnum}+", "a1é");
    t("\\p{Punct}+", "!/:@[`{~");
    t("\\p{Punct}", "é");
    t("\\p{Graph}", " ");
    t("\\p{Print}", "\u007f");
    t("\\p{Cntrl}", "\u007f");
    t("\\p{Blank}", "\t");
    t("\\p{Space}", "\u000b");
    t("\\p{XDigit}+", "fF9g");
    t("\\p{ASCII}", "é");
    t("\\p{Lower}", "é");
    t("\\p{Upper}", "A");
    t("\\p{L}", "é");
    t("\\p{L}", internal("é"));
    t("\\p{IsL}", "é");
    t("\\pL", "é");
    t("\\PL", "é");
    t("\\p{Lu}", "É");
    t("\\p{IsLu}", "É");
    t("\\p{gc=Nd}", "٣");
    t("\\p{general_category=Lu}", "A");
    t("\\p{N}", "²");
    t("\\p{Sc}", "€");
    t("\\p{LC}", "a");
    t("\\p{LD}", "٣");
    t("\\p{L1}", "ÿ");
    t("\\p{L1}", "Ā");
    t("\\p{all}", "\n");
    t("\\p{IsAlphabetic}", "é");
    t("\\p{IsWhite_Space}", " ");
    t("\\p{IsLowercase}", "ª");
    t("\\p{IsUppercase}", "A");
    t("\\p{Foo}", "a");
    t("\\p{}", "a");
    t("\\p{L", "a");
    t("\\p{foo=bar}", "a");
    loud("\\p{javaLowerCase}", "é");
    loud("\\p{InGreek}", "α");
    loud("\\p{IsLatin}", "a");
    loud("\\p{sc=Latin}", "a");
    t("\\p{IsAlpha}", "é");
    t("\\p{IsLetter}", "é");
    t("\\p{IsEmoji}", "🌱");
    t("\\p{IsEmoji}", "1");
    t("\\p{IsEmoji}", "a");
    t("\\p{IsEmoji_Presentation}", "🌱");
    t("\\p{IsExtended_Pictographic}", "©");
    t("\\p{IsIdeographic}", "中");
    t("\\p{IsPunctuation}", "!");
    t("\\p{IsHex_Digit}", "\uff21");
    t("\\p{IsAssigned}", "\u0378");
    t("\\p{IsTitlecase}", "ǅ");
    t("(?i)\\p{IsLowercase}", "A");
    t("\\p{IsJoin_Control}", "\u200d");
    t("\\p{IsPrint}", "\u0085");
    t("\\p{IsNoncharacterCodePoint}", "\ufdd0");
    t("\\p{IsWord}", "_");
    t("\\p{IsControl}", "\u0007");
    t("\\p{L}{7}\\p{IsEmoji}", "äöüÄÖÜß🌱");
    // Anchors.
    t("^a", "a");
    t("^a", "ba");
    t("a$", "a");
    t("a$", "ab");
    loud("a$", "a\n");
    loud("a$", "a\r\n");
    loud("a$\\n", "a\n");
    t("a$", "a\nb");
    t("\\Aa\\z", "a");
    t("a\\z", "ab");
    t("a\\Z", "a");
    loud("a\\Z", "a\n");
    t("(?m)^b", "a\nb");
    t("(?m)a$", "a\nb");
    t("^b", M, "a\nb");
    loud("(?m)^", "");
    loud("(?m)^$", "");
    loud("(?m)^", "a\n");
    loud("(?m)a$", "a\r\n");
    loud("(?m)^b", "a\rb");
    t("(?m)$", "");
    t("^*a", "a");
    // Groups and flags.
    t("(a)(b)", "ab");
    t("(?<n>a)b", "ab");
    t("(?<a1>x)", "x");
    t("(?<a_b>x)", "x");
    t("(?<1a>x)", "x");
    t("(?<n>a)(?<n>b)", "ab");
    t("(?P<n>a)", "a");
    t("(?:a|b)+", "abba");
    t("()", "");
    t("(?:)", "");
    t("(a|)", "");
    t("(?)", "");
    t("(?-)a", "a");
    t("(?$)", "");
    t("(?@)", "");
    t("(a", "a");
    t("a)", "a");
    t("(?i", "a");
    t("(?u)", "");
    loud("(?c)", "");
    loud("(?x)a b", "ab");
    loud("(?d)a.b", "a\rb");
    t("(?U)\\w", "é");
    t("(?U)\\w+", internal("é"));
    t("(?U)\\w+", "a_٣\u0301");
    t("(?U)\\W", "-");
    t("(?U)\\d", "٣");
    t("(?U)\\D", "٣");
    t("(?U)\\s", "\u2028");
    t("(?U)\\s", "\u00a0");
    t("(?U)\\S", "\u0085");
    t("(?U)\\bfoo\\b", "éfoo");
    t("(?U)\\bfoo\\b", "-foo-");
    t("(?U)\\Bfoo", "éfoo");
    t("(?U)\\p{Alpha}", "é");
    t("(?U)\\p{alpha}", "é");
    t("(?U)\\p{Punct}", "«");
    t("(?U)\\p{Print}", "é");
    t("(?U)\\p{Print}", "\u0085");
    t("(?U)\\p{Graph}", " ");
    t("(?U)\\p{Blank}", "\u3000");
    t("(?U)\\p{XDigit}", "\uff21");
    t("(?U)\\p{Lower}", "ª");
    t("(?U)(?i)\\p{Lower}", "A");
    t("(?U)\\p{L}", "é");
    t("(?U)[\\w&&[^\\d]]", "٣");
    t("\\w", Pattern.UNICODE_CHARACTER_CLASS, "é");
    // Quantifiers.
    t("a{2}", "aaa");
    t("a{2,}", "aaaa");
    t("a{2,3}", "aaaa");
    t("a{2,3}?", "aaaa");
    t("a{,3}", "a{,3}");
    t("a{2", "a{2");
    t("a{2,x}", "a");
    t("a{3,2}", "aaa");
    t("a{99999999999}", "a");
    t("x{}", "x{}");
    t("{", "{");
    t("{3}x", "x");
    t("a{2}{3}", "aaaaaa");
    t("a]", "a]");
    t("a}", "a}");
    t("a**", "a");
    t("*a", "a");
    t("a|*", "a");
    t("a??b", "ab");
    t("a*?b", "aab");
    t("a+?", "aaa");
    t("abc*", "abccc");
    t("abc{2}", "abcc");
    t("(?:ab){2}", "abab");
    t("(?i)*", "a");
    // Possessive quantifiers and atomic groups: translated only where nothing follows them.
    t("a*+", "aaa");
    t("ba++", "baaa");
    t("x(?:a|b)?+", "xb");
    t("a{1,2}+", "aaa");
    t("(?>a*)", "aaa");
    t("(?>a|ab)", "ab");
    t("(?:(?>a)|(?>a/b))", "xa/b");
    t("(?:(?>^c)|(?>_test$)|(?>a/b))", "foo_test");
    t("(?:(?>^c)|(?>_test$)|(?>a/b))", "abc");
    loud("(?>a*)a", "aaa");
    loud("a*+a", "aaa");
    loud("(?>a)+", "aa");
    loud("(?:(?>a)b)", "ab");
    loud("(?>ab|a)c", "abc");
    loud("a++b", "aab");
    // Lookahead; lookbehind of a fixed length (TreeInfo minLength == maxLength).
    t("(?!Genrule).*", "Genrule");
    t("(?!Genrule).*", "CppCompile");
    t("(?!CppCompile).*", "CppCompile");
    t("(?!CppCompile).*", "Genrule");
    t("(?=a)", "ba");
    t("(?=a)a", "ba");
    t("foo(?=bar)", "foobaz foobar");
    t("foo(?!bar)", "foobar foobaz");
    t("(?=(a+))a*b", "aab");
    t("(a(?=b))+", "ababx");
    t("(?i)(?=FOO)foo", "xfoo");
    t("x(?=\\s|$)", "ax x");
    t("^(?!.*bar).*$", "foobaz");
    t("^(?!.*bar).*$", "foobar");
    t("(?:(?!ab).)*", "xxabyy");
    t("(?=a)*b", "b");
    t("(?=a", "a");
    t("(?<=a)b", "cab");
    t("(?<!a)b", "ab cb");
    t("(?<=ab|cd)x", "cdx");
    t("(?<=a{2})b", "aab");
    t("(?<=\\d{3})x", "12x 123x");
    t("(?<!^)x", "xx");
    t("(?<=(?:ab){2})c", "ababc");
    // Look-behind group does not have an obvious maximum length (JDK 25's TreeInfo: a repeated
    // group overflows GroupCurly's arithmetic or is a Loop).
    t("(?<=(ab)+)x", "ababx");
    t("(?<=(?:a|bc)+)x", "abcx");
    t("(?<=xa+?)y", "xay");
    // Lookbehind of variable length: not ported (fancy-regex has fixed-length lookbehind).
    loud("(?<=a+)b", "aab");
    loud("(?<=a*)b", "b");
    loud("(?<=a|bc)x", "bcx");
    loud("(?<=a{1,3})b", "aaab");
    loud("(?<=.*)x", "abx");
    loud("(?<=a+b*)x", "aabx");
    loud("(?<!a+)b", "aab");
    // Lookbehind on an input with a supplementary char (Java steps back by UTF-16 units).
    loud("(?<=.)x", "\uD83D\uDE00x");
    // Not ported: backreferences, \G, \X, \b{g}, \N{..}.
    loud("(a)\\1", "aa");
    loud("\\1", "x");
    loud("(?<n>a)\\k<n>", "aa");
    t("\\k<n>", "a");
    t("\\kn", "a");
    t("a(?=b)", "ab");
    t("a(?!b)", "ac");
    t("(?<=a)b", "ab");
    t("(?<!a)b", "cb");
    loud("\\G", "a");
    loud("\\X", "a");
    loud("\\b{g}", "a");
    loud("\\N{LATIN SMALL LETTER A}", "a");
    t("\\N{LATIN SMALL LETTER A", "a");
    t("\\Nx", "a");
    t("\\b{x}", "a");
    loud("\\uD83D", "😀");
    // LITERAL.
    t("a.b", L, "a.b");
    t("a.b", L, "axb");
    t("A.b", L | I, "a.B");
    t("\\Q", L, "\\Q");
    // Patterns used by Bazel.
    t("bazel-out/.*\\.o", internal("bazel-out/Ån.o"));
    t(".*\\.(cc|h)$", "foo/bar.cc");
    t("^//foo", "//foo/bar");
    t("Genrule", "Genrule");
    t("(?:(?>-/javatests[/:])|(?>-/test/java[/:]))", "//javatests/foo");
    t(".*", "");
    t("", "abc");
  }
}
