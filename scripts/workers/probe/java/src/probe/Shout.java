package probe;

/** Depends on Greeter (a second Javac action, through the header jar). */
public final class Shout {
  private Shout() {}

  public static String shout(String name) {
    return Greeter.greet(name).toUpperCase(java.util.Locale.ROOT);
  }
}
