/*
 * In-process microbenchmarks. Each test runs a short warm-up and then N
 * timed iterations inside one process, so enclave/runtime startup is not
 * part of the per-operation numbers.
 *
 * Usage: microbench [syscall_iters] [proc_iters] [test...]
 *   syscall_iters  iterations for getpid/clock/pipe tests (default 100000);
 *                  file tests use a tenth of this
 *   proc_iters     iterations for fork/spawn tests (default 20)
 *   test...        subset of tests to run (default: all)
 *
 * Output, one line per test (grep for the "@mb" prefix):
 *   @mb <test> <iters> <total_ns> <ns_per_op>
 *   @mb <test> skip <reason> (errno <n>)
 *
 * Build with -DNO_FORK where fork() is unavailable (Occlum); the fork tests
 * are then skipped and pipe_pingpong uses posix_spawn instead.
 *
 * Environment: MB_FILE overrides the scratch file (default /tmp/microbench.dat);
 * MB_BLOCKS sets how many blocks the file tests use before wrapping (default 256).
 */
#include <errno.h>
#include <fcntl.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;

#define EXEC_TARGET "automated_tests/nop"

static const char *self_path;
static const char *file_path = "/tmp/microbench.dat";
static char buf[4096];
static volatile long sink;

static long long now_ns(void)
{
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

/* Every test returns 0 on success or an errno value. */

static int t_getpid(long n)
{
  for (long i = 0; i < n; i++)
    sink += getpid();
  return 0;
}

static int t_clock(long n)
{
  struct timespec ts;
  for (long i = 0; i < n; i++) {
    clock_gettime(CLOCK_MONOTONIC, &ts);
    sink += ts.tv_nsec;
  }
  return 0;
}

/*
 * File tests wrap around every file_blocks blocks (1 MB at 4 KB by default),
 * so they measure the per-call cost rather than how each in-memory
 * filesystem copes with a growing file (Occlum's ramfs lives in a 32 MB
 * LibOS heap).
 */
static long file_blocks = 256;

static int file_write(long n, size_t sz)
{
  int fd = open(file_path, O_CREAT | O_TRUNC | O_WRONLY, 0600);
  if (fd < 0)
    return errno;
  int ret = 0;
  for (long i = 0; i < n; i++) {
    if (i % file_blocks == 0 && lseek(fd, 0, SEEK_SET) < 0) {
      ret = errno;
      break;
    }
    if (write(fd, buf, sz) != (ssize_t)sz) {
      ret = errno ? errno : EIO;
      break;
    }
  }
  close(fd);
  return ret;
}

/* Times only the reads; the file is filled first with untimed writes. */
static int file_read(long n, size_t sz, long long *elapsed)
{
  int ret = file_write(file_blocks, sz);
  if (ret)
    return ret;
  int fd = open(file_path, O_RDONLY);
  if (fd < 0)
    return errno;
  long long s = now_ns();
  for (long i = 0; i < n; i++) {
    if (i % file_blocks == 0 && lseek(fd, 0, SEEK_SET) < 0) {
      ret = errno;
      break;
    }
    if (read(fd, buf, sz) != (ssize_t)sz) {
      ret = errno ? errno : EIO;
      break;
    }
  }
  *elapsed = now_ns() - s;
  close(fd);
  return ret;
}

static int t_file_write_64(long n) { return file_write(n, 64); }
static int t_file_write_4k(long n) { return file_write(n, 4096); }

/* lseek on an open file: a file call that copies no data. */
static int t_file_lseek(long n)
{
  int fd = open(file_path, O_CREAT | O_TRUNC | O_RDWR, 0600);
  if (fd < 0)
    return errno;
  int ret = 0;
  for (long i = 0; i < n; i++) {
    if (lseek(fd, 0, SEEK_SET) < 0) {
      ret = errno;
      break;
    }
  }
  close(fd);
  return ret;
}

/* Single-process pipe: write then read one byte. */
static int t_pipe_rw(long n)
{
  int p[2];
  if (pipe(p) < 0)
    return errno;
  int ret = 0;
  for (long i = 0; i < n; i++) {
    if (write(p[1], buf, 1) != 1 || read(p[0], buf, 1) != 1) {
      ret = errno ? errno : EIO;
      break;
    }
  }
  close(p[0]);
  close(p[1]);
  return ret;
}

static void pingpong_child(int rfd, int wfd, long n)
{
  char c;
  for (long i = 0; i < n; i++)
    if (read(rfd, &c, 1) != 1 || write(wfd, &c, 1) != 1)
      _exit(1);
  _exit(0);
}

/* Two processes bounce one byte over a pair of pipes; one op = one round trip. */
static int t_pipe_pingpong(long n)
{
  int to_child[2], to_parent[2];
  if (pipe(to_child) < 0 || pipe(to_parent) < 0)
    return errno;
  pid_t pid;
#ifdef NO_FORK
  char rfd[16], wfd[16], cnt[24];
  snprintf(rfd, sizeof(rfd), "%d", to_child[0]);
  snprintf(wfd, sizeof(wfd), "%d", to_parent[1]);
  snprintf(cnt, sizeof(cnt), "%ld", n);
  char *argv[] = {(char *)self_path, "--pp-child", rfd, wfd, cnt, NULL};
  int err = posix_spawn(&pid, self_path, NULL, NULL, argv, environ);
  if (err)
    return err;
#else
  pid = fork();
  if (pid < 0)
    return errno;
  if (pid == 0) {
    close(to_child[1]);
    close(to_parent[0]);
    pingpong_child(to_child[0], to_parent[1], n);
  }
#endif
  close(to_child[0]);
  close(to_parent[1]);
  int ret = 0;
  for (long i = 0; i < n; i++) {
    if (write(to_child[1], buf, 1) != 1 || read(to_parent[0], buf, 1) != 1) {
      ret = errno ? errno : EIO;
      break;
    }
  }
  close(to_child[1]);
  close(to_parent[0]);
  int status;
  waitpid(pid, &status, 0);
  if (!ret && !(WIFEXITED(status) && WEXITSTATUS(status) == 0))
    ret = ECHILD;
  return ret;
}

static int wait_ok(pid_t pid)
{
  int status;
  if (waitpid(pid, &status, 0) < 0)
    return errno;
  return WIFEXITED(status) && WEXITSTATUS(status) == 0 ? 0 : ECHILD;
}

#ifndef NO_FORK
static int t_fork_exit(long n)
{
  for (long i = 0; i < n; i++) {
    pid_t pid = fork();
    if (pid < 0)
      return errno;
    if (pid == 0)
      _exit(0);
    int ret = wait_ok(pid);
    if (ret)
      return ret;
  }
  return 0;
}

static int t_fork_exec(long n)
{
  char *argv[] = {"nop", NULL};
  for (long i = 0; i < n; i++) {
    pid_t pid = fork();
    if (pid < 0)
      return errno;
    if (pid == 0) {
      execv(EXEC_TARGET, argv);
      _exit(127);
    }
    int ret = wait_ok(pid);
    if (ret)
      return ret;
  }
  return 0;
}
#endif

static int t_spawn_exec(long n)
{
  char *argv[] = {"nop", NULL};
  for (long i = 0; i < n; i++) {
    pid_t pid;
    int err = posix_spawn(&pid, EXEC_TARGET, NULL, NULL, argv, environ);
    if (err)
      return err;
    int ret = wait_ok(pid);
    if (ret)
      return ret;
  }
  return 0;
}

enum kind { SYSCALL, FILEIO, PROC };

struct test {
  const char *name;
  int (*fn)(long);
  enum kind kind;
  size_t read_size; /* nonzero: read test with its own timing */
};

static const struct test tests[] = {
  {"getpid", t_getpid, SYSCALL, 0},
  {"clock_gettime", t_clock, SYSCALL, 0},
  {"pipe_rw", t_pipe_rw, SYSCALL, 0},
  {"pipe_pingpong", t_pipe_pingpong, SYSCALL, 0},
  {"file_lseek", t_file_lseek, FILEIO, 0},
  {"file_write_64", t_file_write_64, FILEIO, 0},
  {"file_write_4k", t_file_write_4k, FILEIO, 0},
  {"file_read_64", NULL, FILEIO, 64},
  {"file_read_4k", NULL, FILEIO, 4096},
#ifdef NO_FORK
  {"fork_exit", NULL, PROC, 0},
  {"fork_exec", NULL, PROC, 0},
#else
  {"fork_exit", t_fork_exit, PROC, 0},
  {"fork_exec", t_fork_exec, PROC, 0},
#endif
  {"spawn_exec", t_spawn_exec, PROC, 0},
};

static int run_once(const struct test *t, long n, long long *elapsed)
{
  if (t->read_size)
    return file_read(n, t->read_size, elapsed);
  long long s = now_ns();
  int ret = t->fn(n);
  *elapsed = now_ns() - s;
  return ret;
}

static void run_test(const struct test *t, long n)
{
  if (!t->fn && !t->read_size) {
    printf("@mb %s skip no-fork\n", t->name);
    fflush(stdout);
    return;
  }
  long long elapsed;
  long warm = n / 10 > 0 ? n / 10 : 1;
  int ret = run_once(t, warm, &elapsed);
  if (!ret)
    ret = run_once(t, n, &elapsed);
  if (ret)
    printf("@mb %s skip %s (errno %d)\n", t->name, strerror(ret), ret);
  else
    printf("@mb %s %ld %lld %.1f\n", t->name, n, elapsed, (double)elapsed / n);
  fflush(stdout);
}

int main(int argc, char **argv)
{
  if (argc == 5 && strcmp(argv[1], "--pp-child") == 0)
    pingpong_child(atoi(argv[2]), atoi(argv[3]), atol(argv[4]));

  self_path = argv[0];
#ifdef NO_FORK
  /* Occlum passes only the basename as argv[0]; its binaries live in /bin. */
  static char self_buf[256];
  if (!strchr(argv[0], '/')) {
    snprintf(self_buf, sizeof(self_buf), "/bin/%s", argv[0]);
    self_path = self_buf;
  }
#endif
  if (getenv("MB_FILE"))
    file_path = getenv("MB_FILE");
  if (getenv("MB_BLOCKS") && atol(getenv("MB_BLOCKS")) > 0)
    file_blocks = atol(getenv("MB_BLOCKS"));
  long sys_iters = argc > 1 ? atol(argv[1]) : 100000;
  long proc_iters = argc > 2 ? atol(argv[2]) : 20;
  memset(buf, 'x', sizeof(buf));

  for (size_t i = 0; i < sizeof(tests) / sizeof(tests[0]); i++) {
    const struct test *t = &tests[i];
    if (argc > 3) {
      int wanted = 0;
      for (int a = 3; a < argc; a++)
        wanted |= strcmp(argv[a], t->name) == 0;
      if (!wanted)
        continue;
    }
    long n = t->kind == PROC ? proc_iters
           : t->kind == FILEIO ? sys_iters / 10 : sys_iters;
    run_test(t, n);
  }
  unlink(file_path);
  return 0;
}
