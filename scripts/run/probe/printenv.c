#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void show(const char *name) {
  const char *v = getenv(name);
  printf("%s=%s\n", name, v ? v : "<unset>");
}

int main(int argc, char **argv) {
  char cwd[4096];
  for (int i = 1; i < argc; i++) printf("arg[%d]=%s\n", i, argv[i]);
  show("BUILD_WORKSPACE_DIRECTORY");
  show("BUILD_WORKING_DIRECTORY");
  show("BUILD_EXECROOT");
  const char *id = getenv("BUILD_ID");
  printf("BUILD_ID length=%zu\n", id ? strlen(id) : (size_t)0);
  show("RUN_ENV_PROBE");
  show("RUNFILES_DIR");
  show("JAVA_RUNFILES");
  show("RUNFILES_MANIFEST_FILE");
  show("RUNFILES_MANIFEST_ONLY");
  show("TEST_SRCDIR");
  show("TEST_TMPDIR");
  show("TEST_WORKSPACE");
  show("TEST_TARGET");
  show("XML_OUTPUT_FILE");
  show("TEST_BINARY");
  show("PWD");
  if (getcwd(cwd, sizeof cwd)) printf("cwd=%s\n", cwd);
  FILE *f = fopen("blaze_run_probe/data.txt", "r");
  printf("runfile blaze_run_probe/data.txt %s\n", f ? "readable" : "missing");
  if (f) fclose(f);
  return 0;
}
