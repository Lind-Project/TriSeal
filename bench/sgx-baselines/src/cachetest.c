/*
 * Exec module cache check: rewrite one path with programs of the same size
 * and exec it after each rewrite. Every exec must run the current contents.
 * Usage: cachetest <progA> <progB> <path>   (src/printa.c, src/printb.c)
 * Output: what each exec printed, then "@cache ok" or "@cache FAIL".
 */
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

static char buf[8 << 20];

static int copy(const char *from, const char *to)
{
  int in = open(from, O_RDONLY), out = open(to, O_CREAT | O_TRUNC | O_WRONLY, 0755);
  ssize_t n, total = 0;
  if (in < 0 || out < 0)
    return -1;
  while ((n = read(in, buf, sizeof(buf))) > 0)
    total += write(out, buf, n);
  close(in);
  close(out);
  return (int)total;
}

/* Exec path in a child, then read what it recorded. The helpers write to a
 * file because the imfs grate passes fds 0-2 straight to the host, so
 * stdout cannot be redirected into IMFS. */
static void run(const char *path, char *got, size_t len)
{
  const char *out = "/tmp/cachetest.out";
  unlink(out);
  pid_t pid = fork();
  if (pid == 0) {
    char *argv[] = {(char *)path, NULL};
    execv(path, argv);
    _exit(127);
  }
  waitpid(pid, NULL, 0);
  int fd = open(out, O_RDONLY);
  ssize_t n = fd < 0 ? -1 : read(fd, got, len - 1);
  got[n > 0 ? n : 0] = 0;
  if (fd >= 0)
    close(fd);
}

int main(int argc, char **argv)
{
  setvbuf(stdout, NULL, _IONBF, 0);
  if (argc != 4)
    return 2;
  const char *want[] = {"AAAA", "BBBB", "BBBB"};
  char got[3][64];
  copy(argv[1], argv[3]);
  run(argv[3], got[0], sizeof(got[0]));
  copy(argv[2], argv[3]);
  run(argv[3], got[1], sizeof(got[1]));
  run(argv[3], got[2], sizeof(got[2]));
  int ok = 1;
  for (int i = 0; i < 3; i++) {
    printf("exec %d printed %.4s (want %s)\n", i, got[i], want[i]);
    ok &= strncmp(got[i], want[i], 4) == 0;
  }
  printf("@cache %s\n", ok ? "ok" : "FAIL");
  return !ok;
}
