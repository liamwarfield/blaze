package probe.app;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import probe.Greeter;

public final class Main {
  public static void main(String[] args) throws Exception {
    System.out.println(Greeter.greet(Annotated.name()));
    try (BufferedReader r = new BufferedReader(new InputStreamReader(
        Main.class.getResourceAsStream("/probe/greeting.txt"), StandardCharsets.UTF_8))) {
      System.out.println(r.readLine());
    }
    System.out.println(Class.forName("probe.Runtime").getSimpleName());
  }
}
