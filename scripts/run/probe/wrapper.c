#include <stdio.h>
#include <unistd.h>
int main(int argc, char **argv) {
  printf("wrapper: %d args\n", argc - 1);
  fflush(stdout);
  if (argc < 2) return 0;
  execv(argv[1], argv + 1);
  perror("wrapper: execv");
  return 127;
}
