// What the JDK's TLS trust manager decides for a server certificate, step by step as
// sun.security.ssl.X509TrustManagerImpl.checkTrusted does it for a client:
//   1. CertificateFactory "X.509" parses the DER (X509Factory -> X509CertImpl -> X509CertInfo),
//   2. the PKIX trust manager over the given roots validates the one-certificate chain
//      (Validator.TYPE_PKIX -> PKIXValidator, then the EndEntityChecker),
//   3. endpoint identification "HTTPS" as X509TrustManagerImpl.checkIdentity(session, ...) does it
//      for Bazel's channel (no SNI host name for a name SNIHostName refuses, so the peer host): one
//      trailing dot removed from the host, then HostnameChecker.getInstance(TYPE_TLS).match(host,
//      cert, chainsToPublicCA) with chainsToPublicCA = AnchorCertificates.contains(anchor).
// Usage: java --add-exports java.base/sun.security.util=ALL-UNNAMED \
//             --add-exports java.base/sun.security.validator=ALL-UNNAMED JdkCertProbe <dir>
// Reads <dir>/cases.tsv, prints one line per case: case<TAB>ACCEPT|REJECT<TAB>step: message.

import java.io.ByteArrayInputStream;
import java.io.FileInputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.KeyStore;
import java.security.cert.Certificate;
import java.security.cert.CertificateFactory;
import java.security.cert.X509Certificate;
import java.util.Collection;
import java.util.List;
import javax.net.ssl.TrustManager;
import javax.net.ssl.TrustManagerFactory;
import javax.net.ssl.X509TrustManager;
import sun.security.util.AnchorCertificates;
import sun.security.util.HostnameChecker;

public class JdkCertProbe {
  public static void main(String[] args) throws Exception {
    if (args.length != 1 || args[0].equals("help")) {
      System.out.println("usage: JdkCertProbe <fixture dir with cases.tsv>");
      return;
    }
    Path dir = Path.of(args[0]);
    List<String> lines = Files.readAllLines(dir.resolve("cases.tsv"));
    System.out.println("# case\tverdict\tstep: message (" + System.getProperty("java.vendor")
        + " " + System.getProperty("java.runtime.version") + ")");
    for (String line : lines) {
      if (line.startsWith("#") || line.isEmpty()) {
        continue;
      }
      String[] f = line.split("\t");
      System.out.println(f[0] + "\t" + probe(dir, f[0], f[1], f[2]));
    }
  }

  static String probe(Path dir, String name, String roots, String host) {
    CertificateFactory cf;
    X509Certificate cert;
    try {
      cf = CertificateFactory.getInstance("X.509");
      byte[] der = Files.readAllBytes(dir.resolve(name + ".der"));
      cert = (X509Certificate) cf.generateCertificate(new ByteArrayInputStream(der));
    } catch (Exception e) {
      return "REJECT\tparse: " + e;
    }
    X509Certificate anchor;
    try {
      KeyStore ks = KeyStore.getInstance(KeyStore.getDefaultType());
      ks.load(null, null);
      Collection<? extends Certificate> rootCerts;
      try (FileInputStream in = new FileInputStream(dir.resolve(roots).toFile())) {
        rootCerts = cf.generateCertificates(in);
      }
      int i = 0;
      for (Certificate c : rootCerts) {
        ks.setCertificateEntry("root" + i++, c);
      }
      TrustManagerFactory tmf = TrustManagerFactory.getInstance("PKIX");
      tmf.init(ks);
      X509TrustManager tm = null;
      for (TrustManager t : tmf.getTrustManagers()) {
        if (t instanceof X509TrustManager) {
          tm = (X509TrustManager) t;
        }
      }
      tm.checkServerTrusted(new X509Certificate[] {cert}, "UNKNOWN");
      anchor = (X509Certificate) rootCerts.iterator().next();
    } catch (Exception e) {
      return "REJECT\ttrust: " + e;
    }
    try {
      boolean chainsToPublicCA = AnchorCertificates.contains(anchor);
      // X509TrustManagerImpl.checkIdentity: "Remove the ending dot, which is not allowed in
      // SNIHostName."
      if (host.endsWith(".")) {
        host = host.substring(0, host.length() - 1);
      }
      HostnameChecker.getInstance(HostnameChecker.TYPE_TLS).match(host, cert, chainsToPublicCA);
    } catch (Exception e) {
      return "REJECT\thost: " + e;
    }
    return "ACCEPT\t";
  }
}
