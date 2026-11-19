/* Test program of the probe package of scripts/tests/compare. The first argument is the mode. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void touch(const char *var) {
  const char *path = getenv(var);
  if (path != NULL) {
    FILE *f = fopen(path, "w");
    if (f != NULL) fclose(f);
  }
}

static int write_xml(int failing) {
  const char *path = getenv("XML_OUTPUT_FILE");
  if (path == NULL) return 0;
  FILE *f = fopen(path, "w");
  if (f == NULL) return 0;
  fprintf(f,
          "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
          "<testsuites name=\"AllTests\" time=\"0.25\">\n"
          "  <testsuite name=\"ProbeSuite\" time=\"0.2\">\n"
          "    <testcase name=\"first\" classname=\"ProbeSuite\" time=\"0.1\" status=\"run\" "
          "result=\"completed\"/>\n"
          "    <testcase name=\"second\" classname=\"ProbeSuite\" time=\"12\" status=\"run\">\n"
          "%s"
          "    </testcase>\n"
          "    <testcase name=\"third\" classname=\"ProbeSuite\" time=\"1e-2\" status=\"notrun\">"
          "<skipped/></testcase>\n"
          "  </testsuite>\n"
          "</testsuites>\n",
          failing ? "      <failure message=\"boom\">details &amp; more</failure>\n" : "");
  fclose(f);
  return 0;
}

int main(int argc, char **argv) {
  const char *mode = argc > 1 ? argv[1] : "pass";
  const char *name = getenv("TEST_TARGET");
  printf("probe %s %s\n", mode, name ? name : "?");
  fflush(stdout);
  if (strcmp(mode, "pass") == 0) return 0;
  if (strcmp(mode, "fail") == 0) return 1;
  if (strcmp(mode, "sleep") == 0) {
    sleep(argc > 2 ? atoi(argv[2]) : 1);
    printf("probe slept\n");
    return 0;
  }
  if (strcmp(mode, "lines") == 0) {
    /* Several lines with pauses, for --test_output=streamed. */
    for (int i = 0; i < 3; i++) {
      printf("probe line %d\n", i);
      fflush(stdout);
      usleep(200 * 1000);
    }
    return 0;
  }
  if (strcmp(mode, "shard") == 0) {
    touch("TEST_SHARD_STATUS_FILE");
    printf("shard %s of %s\n", getenv("TEST_SHARD_INDEX") ? getenv("TEST_SHARD_INDEX") : "-",
           getenv("TEST_TOTAL_SHARDS") ? getenv("TEST_TOTAL_SHARDS") : "-");
    return 0;
  }
  if (strcmp(mode, "xml-pass") == 0) return write_xml(0);
  if (strcmp(mode, "xml-fail") == 0) {
    write_xml(1);
    return 1;
  }
  fprintf(stderr, "unknown mode %s\n", mode);
  return 2;
}
