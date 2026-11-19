package probe;

public final class GreeterTest {
  public static void main(String[] args) {
    String got = Greeter.greet("test");
    if (!got.equals("Hello, test!")) {
      System.err.println("FAIL: " + got);
      System.exit(1);
    }
    System.out.println("PASS");
  }
}
