package probe.proc;

import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/** Marks a class for which ProbeProcessor generates a companion. */
@Retention(RetentionPolicy.SOURCE)
@Target(ElementType.TYPE)
public @interface Gen {}
