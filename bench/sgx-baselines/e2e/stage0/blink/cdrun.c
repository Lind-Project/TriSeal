// Usage: cdrun DIR PROG [ARGS...]. Lind cages start in /, so chdir before exec.
#include <stdio.h>
#include <unistd.h>

int main(int argc, char **argv) {
  if (argc < 3) {
    fprintf(stderr, "usage: cdrun DIR PROG [ARGS...]\n");
    return 2;
  }
  if (chdir(argv[1])) {
    perror("cdrun: chdir");
    return 1;
  }
  execv(argv[2], argv + 2);
  perror("cdrun: execv");
  return 127;
}
