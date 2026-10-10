/*
 * Build the same tree several times in one TEE instance and time each
 * build, to separate a cold build (the first one after the TEE starts)
 * from a warm one (a later build in the running instance).
 *
 * Usage: twobuild <make> <dir>... -- <make args...>
 * Runs `<make> -C <dir> <make args...>` for each dir in turn.
 * Output: "@build <n> <dir> <seconds> <exit status>" per dir.
 */
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define MAX_ARGS 64

static double now(void)
{
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return ts.tv_sec + ts.tv_nsec / 1e9;
}

int main(int argc, char **argv)
{
  /* Unbuffered: lind does not flush stdio buffers when a cage exits. */
  setvbuf(stdout, NULL, _IONBF, 0);

  int sep = 0;
  for (int i = 2; i < argc; i++) {
    if (strcmp(argv[i], "--") == 0) {
      sep = i;
      break;
    }
  }
  int nargs = argc - sep - 1;
  if (sep < 3 || nargs + 4 > MAX_ARGS) {
    fprintf(stderr, "usage: twobuild <make> <dir>... -- <make args...>\n");
    return 2;
  }

  int worst = 0;
  for (int d = 2; d < sep; d++) {
    char *av[MAX_ARGS];
    int k = 0;
    av[k++] = argv[1];
    av[k++] = "-C";
    av[k++] = argv[d];
    for (int j = 0; j < nargs; j++)
      av[k++] = argv[sep + 1 + j];
    av[k] = NULL;

    double start = now();
    pid_t pid = fork();
    if (pid == 0) {
      execv(argv[1], av);
      _exit(127);
    }
    int status = 0;
    waitpid(pid, &status, 0);
    int code = WIFEXITED(status) ? WEXITSTATUS(status) : 128;
    printf("@build %d %s %.3f %d\n", d - 1, argv[d], now() - start, code);
    if (code)
      worst = code;
  }
  return worst;
}
