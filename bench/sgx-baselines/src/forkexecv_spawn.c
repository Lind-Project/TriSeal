/*
 * posix_spawn variant of forkexecv.c for Occlum, which has no fork().
 * Same observable behaviour: start automated_tests/hello and wait for it.
 */
#include <spawn.h>
#include <stdio.h>
#include <sys/wait.h>

extern char **environ;

int main(void)
{
  pid_t pid;
  char *arr[] = {"hello", NULL};
  int ret = posix_spawn(&pid, "automated_tests/hello", NULL, NULL, arr, environ);
  if (ret != 0) {
    fprintf(stderr, "posix_spawn error: %d\n", ret);
    return 1;
  }
  waitpid(pid, NULL, 0);
  return 0;
}
