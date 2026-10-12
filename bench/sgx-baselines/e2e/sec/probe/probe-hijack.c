/* Confirm the severity of the register_handler cross-cage finding: does a
 * handler a cage registers for ANOTHER cage's SYS_write actually take effect?
 * The child writes a marker line repeatedly; the parent hijacks the child's
 * SYS_write midway (pointing it at the parent's fake_handler, which returns 0).
 * If the hijack takes effect, the child's later markers stop appearing (or the
 * keep faults). If register_handler was a no-op, all markers appear.
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <sys/wait.h>

extern int register_handler(uint64_t targetcage, uint64_t targetcallnum,
    uint64_t this_grate_id, uint64_t in_grate_fn_ptr_u64);
int fake_handler(void) { return 0; }

int main(void) {
  uint64_t self = (uint64_t)getpid();
  pid_t child = fork();
  if (child == 0) {
    for (int i = 0; i < 8; i++) {
      char m[48]; int n = snprintf(m, sizeof m, "CHILD-MARKER %d (child cage write)\n", i);
      write(1, m, n);        /* raw SYS_write so it is the hijack target */
      sleep(1);
    }
    _exit(0);
  }
  printf("[probe] parent=%llu child=%llu; hijacking child SYS_write after 3 markers\n",
         (unsigned long long)self, (unsigned long long)child);
  fflush(stdout);
  sleep(3);
  int r = register_handler((uint64_t)child, 1, self, (uint64_t)(uintptr_t)&fake_handler);
  printf("[probe] register_handler(child, SYS_write) ret=%d\n", r); fflush(stdout);
  int st; waitpid(child, &st, 0);
  printf("[probe] child exited status=%d; done\n", WEXITSTATUS(st)); fflush(stdout);
  return 0;
}
