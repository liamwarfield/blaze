package probe;

/** A trivial library compiled by Javac in a persistent worker. */
public final class Greeter {
  private Greeter() {}

  public static String greet(String name) {
    return "Hello, " + name + "!";
  }
}
