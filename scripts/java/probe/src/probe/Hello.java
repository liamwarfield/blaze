package probe;

public final class Hello {
  public static void main(String[] args) {
    System.out.println(Greeter.greet(args.length > 0 ? args[0] : "world"));
  }
}
