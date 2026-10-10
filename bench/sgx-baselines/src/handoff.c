/*
 * Cross-process file handoff, the way a compiler pipeline passes files:
 * cc1 writes a temporary .s file that a separately exec'd `as` reads. The
 * parent writes 64 KB to a file, then fork+execs itself to read it back and
 * check every byte.
 *
 * Usage: handoff <path>
 * Output: "@handoff ok <bytes>" or "@handoff fail <reason>".
 */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#define SIZE (64 * 1024)

/* Static: wasm's shadow stack is only about 64 KB. */
static char buf[SIZE];
static char got[SIZE];

static void fill(void)
{
  for (int i = 0; i < SIZE; i++)
    buf[i] = (char)('a' + i % 26);
}

/* Child: read the file back and compare. */
static int check(const char *path)
{
  int fd = open(path, O_RDONLY);
  if (fd < 0) {
    printf("@handoff fail child open: %s\n", strerror(errno));
    return 1;
  }
  ssize_t n = 0, r;
  while (n < SIZE && (r = read(fd, got + n, SIZE - n)) > 0)
    n += r;
  close(fd);
  fill();
  if (n != SIZE || memcmp(got, buf, SIZE) != 0) {
    printf("@handoff fail child read %zd of %d bytes or data differs\n", n, SIZE);
    return 1;
  }
  printf("@handoff ok %d\n", SIZE);
  return 0;
}

int main(int argc, char **argv)
{
  /* Unbuffered: lind does not flush stdio buffers when a cage exits. */
  setvbuf(stdout, NULL, _IONBF, 0);
  if (argc == 3 && strcmp(argv[1], "--check") == 0)
    return check(argv[2]);
  if (argc != 2) {
    fprintf(stderr, "usage: handoff <path>\n");
    return 2;
  }

  fill();
  int fd = open(argv[1], O_CREAT | O_TRUNC | O_WRONLY, 0600);
  if (fd < 0 || write(fd, buf, SIZE) != SIZE) {
    printf("@handoff fail parent write: %s\n", strerror(errno));
    return 1;
  }
  close(fd);

  pid_t pid = fork();
  if (pid == 0) {
    char *args[] = {argv[0], "--check", argv[1], NULL};
    execv(argv[0], args);
    printf("@handoff fail exec %s: %s\n", argv[0], strerror(errno));
    _exit(127);
  }
  int status;
  waitpid(pid, &status, 0);
  unlink(argv[1]);
  return WIFEXITED(status) ? WEXITSTATUS(status) : 1;
}
