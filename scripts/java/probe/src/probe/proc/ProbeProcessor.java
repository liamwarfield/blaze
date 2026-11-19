package probe.proc;

import java.io.IOException;
import java.io.Writer;
import java.util.Set;
import javax.annotation.processing.AbstractProcessor;
import javax.annotation.processing.RoundEnvironment;
import javax.annotation.processing.SupportedAnnotationTypes;
import javax.lang.model.SourceVersion;
import javax.lang.model.element.Element;
import javax.lang.model.element.TypeElement;

/** Generates <Name>Generated with a constant for each class annotated with @Gen. */
@SupportedAnnotationTypes("probe.proc.Gen")
public final class ProbeProcessor extends AbstractProcessor {
  @Override
  public SourceVersion getSupportedSourceVersion() {
    return SourceVersion.latestSupported();
  }

  @Override
  public boolean process(Set<? extends TypeElement> annotations, RoundEnvironment env) {
    for (Element e : env.getElementsAnnotatedWith(Gen.class)) {
      String name = e.getSimpleName() + "Generated";
      try (Writer w = processingEnv.getFiler().createSourceFile("probe.app." + name, e).openWriter()) {
        w.write("package probe.app;\npublic final class " + name
            + " { public static final String NAME = \"" + e.getSimpleName() + "\"; }\n");
      } catch (IOException ex) {
        throw new IllegalStateException(ex);
      }
    }
    return true;
  }
}
