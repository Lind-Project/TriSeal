/* Experiment E for Occlum: are build steps isolated from each other?
 *
 * Occlum runs every process in ONE enclave / one native address space. This
 * benign probe spawns a child process that holds a secret, then — as a
 * compromised build step would — tries to read the child's memory three ways,
 * reporting DENIED / SUCCEEDED(LEAK)/ FAULTED per vector:
 *   1. a plain cross-process pointer read (works iff the address spaces are
 *      shared, i.e. no intra-enclave memory isolation);
 *   2. /proc/<pid>/mem;
 *   3. process_vm_readv.
 * It is the counterpart of the TriSeal cage probe (which uses the 3i
 * primitives). Exfiltrates nothing. Occlum has no fork(), so the child is
 * posix_spawn'd and reports its pid + secret address through /host.
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <signal.h>
#include <setjmp.h>
#include <spawn.h>
#include <sys/wait.h>
#include <sys/uio.h>
#include <sys/syscall.h>
#include <stdint.h>

extern char **environ;
static volatile char secret[64];
static sigjmp_buf jb;
static void onfault(int s) { (void)s; siglongjmp(jb, 1); }
static int has(const char *b, const char *n) { return memmem(b, 64, n, strlen(n)) != NULL; }

int main(int argc, char **argv) {
  if (argc > 1 && !strcmp(argv[1], "child")) {
    strcpy((char *)secret, "OCCLUM-CHILD-SECRET-MUST-NOT-LEAK");
    FILE *f = fopen("/host/child.info", "w");
    if (f) { fprintf(f, "%d %lu\n", getpid(), (unsigned long)(uintptr_t)secret); fclose(f); }
    sleep(8);
    return 0;
  }

  strcpy((char *)secret, "PARENT-placeholder");
  unlink("/host/child.info");
  pid_t cpid; char *av[] = { "/bin/probe-occlum", "child", NULL };
  char *ev[] = { "PATH=/bin", NULL };
  errno = 0;
  int rc = posix_spawn(&cpid, "/bin/probe-occlum", NULL, NULL, av, ev);
  if (rc) { printf("[occlum-E] posix_spawn failed: rc=%d errno=%d (%s)\n", rc, errno, strerror(errno)); return 1; }

  int pid = 0; unsigned long addr = 0;
  for (int i = 0; i < 80; i++) {
    FILE *f = fopen("/host/child.info", "r");
    if (f) { int ok = fscanf(f, "%d %lu", &pid, &addr); fclose(f); if (ok == 2) break; }
    usleep(200000);
  }
  printf("[occlum-E] parent=%d child=%d secret@0x%lx\n", getpid(), pid, addr);
  fflush(stdout);
  if (!pid) { printf("[occlum-E] no child info\n"); return 1; }

  char buf[64];

  /* V1: plain cross-process pointer read. */
  signal(SIGSEGV, onfault); signal(SIGBUS, onfault);
  int leak = 0, faulted = 0;
  memset(buf, 0xA5, sizeof buf);
  if (sigsetjmp(jb, 1) == 0) { memcpy(buf, (void *)(uintptr_t)addr, 64); leak = has(buf, "CHILD-SECRET"); }
  else faulted = 1;
  printf("[occlum-E] V1 direct pointer read: %s leaked=%d -> %s\n",
         faulted ? "FAULTED" : "ok", leak,
         (!faulted && leak) ? "SUCCEEDED(LEAK)" : "DENIED");
  fflush(stdout);
  signal(SIGSEGV, SIG_DFL); signal(SIGBUS, SIG_DFL);

  /* V2: /proc/<pid>/mem. */
  memset(buf, 0xA5, sizeof buf);
  char pp[64]; snprintf(pp, sizeof pp, "/proc/%d/mem", pid);
  int fd = open(pp, O_RDONLY); long r = -1; leak = 0; errno = 0;
  if (fd >= 0) { r = pread(fd, buf, 64, addr); leak = has(buf, "CHILD-SECRET"); }
  printf("[occlum-E] V2 /proc/%d/mem: fd=%d ret=%ld errno=%d leaked=%d -> %s\n",
         pid, fd, r, errno, leak, (r > 0 && leak) ? "SUCCEEDED(LEAK)" : "DENIED");
  if (fd >= 0) close(fd);
  fflush(stdout);

  /* V3: process_vm_readv. */
  memset(buf, 0xA5, sizeof buf);
  struct iovec lo = { buf, 64 }, ro = { (void *)(uintptr_t)addr, 64 }; errno = 0;
  long n = syscall(SYS_process_vm_readv, (long)pid, &lo, 1L, &ro, 1L, 0L);
  leak = has(buf, "CHILD-SECRET");
  printf("[occlum-E] V3 process_vm_readv: n=%ld errno=%d leaked=%d -> %s\n",
         n, errno, leak, (n > 0 && leak) ? "SUCCEEDED(LEAK)" : "DENIED");
  fflush(stdout);

  int st; waitpid(cpid, &st, 0);
  printf("[occlum-E] done\n");
  return 0;
}
