/* Handler-table authorization probe (lind-wasm threei-handler-auth).
 *
 * Each command attempts one way a cage could change routing it does not
 * control. Exit code: 0 = the attempt SUCCEEDED (hole), 1 = DENIED.
 *
 *   hijack-parent  forked child reroutes its parent's SYS_write to itself
 *   hijack-grate   cage reroutes the SYS_write of the cage above it (the
 *                  grate when run under IMFS) to itself
 *   copy-escape    cage copies the table of the cage above it over its own,
 *                  then opens PATH (a host file IMFS does not hold)
 *   native-ptr     cage names RawPOSIX as the handler of its own getpid with
 *                  a made-up native function pointer, then calls getpid
 *   spoof-open     cage claims the id of the cage above it as self_cageid, so
 *                  its open/read of PATH route through that cage's table
 *                  (straight to RawPOSIX when it is the IMFS grate)
 */
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

extern int register_handler(uint64_t targetcage, uint64_t targetcallnum,
                            uint64_t this_grate_id, uint64_t in_grate_fn_ptr_u64);
extern int copy_handler_table_to_cage(uint64_t thiscage, uint64_t targetcage);
extern int make_threei_call(unsigned int callnumber, uint64_t callname,
                            uint64_t self_cageid, uint64_t target_cageid,
                            uint64_t arg1, uint64_t arg1cageid,
                            uint64_t arg2, uint64_t arg2cageid,
                            uint64_t arg3, uint64_t arg3cageid,
                            uint64_t arg4, uint64_t arg4cageid,
                            uint64_t arg5, uint64_t arg5cageid,
                            uint64_t arg6, uint64_t arg6cageid,
                            int translate_errno);
extern uint64_t __lind_base;

#define RAWPOSIX_CAGEID 777777ULL
#define SYS_READ 0
#define SYS_WRITE 1
#define SYS_OPEN 2
#define SYS_GETPID 39
#define REGISTER_HANDLER_SYSCALL 1001
#define NOTUSED 0xdeadbeefdeadbeefULL
#define HOST(p) (__lind_base + (uint64_t)(uintptr_t)(p))

int fake_handler(void) { return 0; }

static int report(const char *cmd, int r) {
  printf("[auth] %s ret=%d (0x%x) -> %s\n", cmd, r, (unsigned)r,
         r == 0 ? "SUCCEEDED" : "DENIED");
  fflush(stdout);
  return r == 0 ? 0 : 1;
}

int main(int argc, char **argv) {
  const char *cmd = argc > 1 ? argv[1] : "";
  uint64_t self = (uint64_t)getpid(), parent = (uint64_t)getppid();
  printf("[auth] cmd=%s self=%llu parent=%llu\n", cmd,
         (unsigned long long)self, (unsigned long long)parent);
  fflush(stdout);

  if (strcmp(cmd, "hijack-parent") == 0) {
    pid_t child = fork();
    if (child == 0) {
      int r = register_handler(self, SYS_WRITE, (uint64_t)getpid(),
                               (uint64_t)(uintptr_t)&fake_handler);
      _exit(r == 0 ? 0 : 1);
    }
    int st = 0;
    waitpid(child, &st, 0);
    int verdict = WEXITSTATUS(st);
    /* Goes to the child's fake_handler if the hijack took effect. */
    printf("[auth] hijack-parent child verdict=%s\n",
           verdict == 0 ? "SUCCEEDED" : "DENIED");
    fflush(stdout);
    return verdict;
  }

  if (strcmp(cmd, "hijack-grate") == 0)
    return report(cmd, register_handler(parent, SYS_WRITE, self,
                                         (uint64_t)(uintptr_t)&fake_handler));

  if (strcmp(cmd, "copy-escape") == 0) {
    const char *path = argc > 2 ? argv[2] : "/etc/hostname";
    int r = copy_handler_table_to_cage(parent, self);
    int fd = open(path, O_RDONLY);
    char buf[64] = {0};
    ssize_t n = fd >= 0 ? read(fd, buf, sizeof buf - 1) : -1;
    if (n > 0 && buf[n - 1] == '\n')
      buf[n - 1] = 0;
    printf("[auth] copy-escape copy ret=%d (0x%x); open(%s)=%d read=%zd \"%s\"\n",
           r, (unsigned)r, path, fd, n, n > 0 ? buf : "");
    fflush(stdout);
    return r == 0 ? 0 : 1;
  }

  if (strcmp(cmd, "native-ptr") == 0) {
    /* glibc's register_handler() passes the handler cage as self_cageid, so
     * make the 1001 call by hand with our own id, as a grate's wrapper would. */
    int r = make_threei_call(REGISTER_HANDLER_SYSCALL, 0, self, self,
                             self, SYS_GETPID,            /* targetcage, callnum */
                             0, RAWPOSIX_CAGEID,          /* runtime, handler cage */
                             0x4141414141ULL, 0,          /* native fn pointer */
                             0, 0, 0, 0, 0, 0, 0);
    printf("[auth] native-ptr ret=%d (0x%x); calling getpid\n", r, (unsigned)r);
    fflush(stdout);
    printf("[auth] native-ptr getpid=%d\n", (int)getpid());
    fflush(stdout);
    return report(cmd, r);
  }

  if (strcmp(cmd, "spoof-open") == 0) {
    const char *path = argc > 2 ? argv[2] : "/etc/hostname";
    int fd = make_threei_call(SYS_OPEN, 0, parent, self, HOST(path), self,
                              O_RDONLY, self, 0, self, NOTUSED, self,
                              NOTUSED, self, NOTUSED, self, 1);
    char buf[64] = {0};
    int n = fd >= 0 ? make_threei_call(SYS_READ, 0, parent, self, fd, self,
                                       HOST(buf), self, sizeof buf - 1, self,
                                       NOTUSED, self, NOTUSED, self, NOTUSED, self, 1)
                    : -1;
    if (n > 0 && buf[n - 1] == '\n')
      buf[n - 1] = 0;
    printf("[auth] spoof-open open(%s) as cage %llu = %d read=%d \"%s\" -> %s\n", path,
           (unsigned long long)parent, fd, n, n > 0 ? buf : "",
           n > 0 ? "SUCCEEDED" : "DENIED");
    fflush(stdout);
    return n > 0 ? 0 : 1;
  }

  printf("[auth] unknown command\n");
  return 2;
}
