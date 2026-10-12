/* Isolation-boundary probe (experiment E), self-contained.
 *
 * A BENIGN probe that forks a child cage holding a secret, then — as a
 * compromised build step would — tries to reach ACROSS the cage boundary into
 * the child via the 3i primitives every cage has (copy_data_between_cages,
 * register_handler). It reports DENIED / SUCCEEDED / FAULTED for each, and
 * exfiltrates nothing. A SUCCEEDED line marks a hole in the cage boundary.
 *
 * Self-contained so it runs bare (enarx run probe.cwasm), where stdout reaches
 * the host; the child is a real sibling cage with its own linear memory.
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <sys/wait.h>

#define ELINDAPIABORTED 0xE0010001u
#define GRATE_MEMORY_FLAG (1ULL << 63)

extern int copy_data_between_cages(uint64_t thiscage, uint64_t targetcage,
    uint64_t srcaddr, uint64_t srccage, uint64_t destaddr, uint64_t destcage,
    uint64_t len, uint64_t copytype);
extern int register_handler(uint64_t targetcage, uint64_t targetcallnum,
    uint64_t this_grate_id, uint64_t in_grate_fn_ptr_u64);

static int denied(int r) { return (unsigned)r == ELINDAPIABORTED || r < 0; }
int fake_handler(void) { return 0; }

/* Secret lives in a fixed global so its guest offset is identical in parent and
 * child after fork; only the owning cage should ever see the child's copy. */
static volatile char secret[64];

int main(void) {
  uint64_t self = (uint64_t)getpid();
  strcpy((char *)secret, "PARENT-placeholder");

  pid_t child = fork();
  if (child == 0) {
    strcpy((char *)secret, "CHILD-SECRET-MUST-NOT-LEAK-ACROSS-CAGES");
    sleep(8);          /* stay alive while the parent probes */
    _exit(0);
  }

  uint64_t caddr = (uint64_t)(uintptr_t)secret;   /* same guest offset in child */
  printf("[probe] parent cage=%llu child cage=%llu secret@0x%llx\n",
         (unsigned long long)self, (unsigned long long)child, (unsigned long long)caddr);
  fflush(stdout);
  sleep(1);            /* let the child write its secret */

  /* P1/P3-read: read the child cage's secret into our buffer. */
  unsigned char buf[64];
  memset(buf, 0xA5, sizeof buf);
  printf("[probe] attempting cross-cage READ of child...\n"); fflush(stdout);
  int r = copy_data_between_cages(self, child, caddr, child,
                                  (uint64_t)(uintptr_t)buf, self, sizeof buf, 0);
  int leaked = (memmem(buf, sizeof buf, "CHILD-SECRET", 12) != NULL);
  printf("[probe] READ child: ret=%d leaked=%d -> %s\n", r, leaked,
         (!denied(r) && leaked) ? "SUCCEEDED(LEAK)" : "DENIED");
  fflush(stdout);

  /* P1-read with the grate-memory translate flag set, in case that bypasses. */
  memset(buf, 0xA5, sizeof buf);
  printf("[probe] attempting cross-cage READ with GRATE_MEMORY_FLAG...\n"); fflush(stdout);
  r = copy_data_between_cages(self, child, caddr, child | GRATE_MEMORY_FLAG,
                              (uint64_t)(uintptr_t)buf, self, sizeof buf, 0);
  leaked = (memmem(buf, sizeof buf, "CHILD-SECRET", 12) != NULL);
  printf("[probe] READ child (flag): ret=%d leaked=%d -> %s\n", r, leaked,
         (!denied(r) && leaked) ? "SUCCEEDED(LEAK)" : "DENIED");
  fflush(stdout);

  /* P3-write: overwrite the child's secret from the parent. */
  static const char payload[32] = "PWNED-BY-PARENT-CAGE";
  printf("[probe] attempting cross-cage WRITE into child...\n"); fflush(stdout);
  r = copy_data_between_cages(self, child, (uint64_t)(uintptr_t)payload, self,
                              caddr, child, sizeof payload, 0);
  printf("[probe] WRITE child: ret=%d -> %s\n", r, denied(r) ? "DENIED" : "SUCCEEDED");
  fflush(stdout);

  /* P2-hijack: register our handler for the child's SYS_write. */
  printf("[probe] attempting to hijack child's SYS_write handler...\n"); fflush(stdout);
  r = register_handler((uint64_t)child, 1, self, (uint64_t)(uintptr_t)&fake_handler);
  printf("[probe] HIJACK child: ret=%d -> %s\n", r, denied(r) ? "DENIED" : "SUCCEEDED");
  fflush(stdout);

  int st; waitpid(child, &st, 0);
  printf("[probe] done\n"); fflush(stdout);
  return 0;
}
