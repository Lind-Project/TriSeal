/* The time canary: every time source a C build commonly reads, compiled
 * into one program. stamp.h is written during the build by `probe time`;
 * the three macros are the compiler's own. */
#include <stdio.h>
#include "stamp.h"

int canary_extra(int x);

int main(void) {
  printf("__DATE__=%s\n", __DATE__);
  printf("__TIME__=%s\n", __TIME__);
  printf("__TIMESTAMP__=%s\n", __TIMESTAMP__);
  printf("time=%s\n", PROBE_TIME);
  printf("gettimeofday=%s\n", PROBE_GETTIMEOFDAY);
  printf("realtime=%s\n", PROBE_REALTIME);
  printf("monotonic=%s\n", PROBE_MONOTONIC);
  printf("mtime_source=%s\n", PROBE_MTIME_0);
  printf("mtime_built=%s\n", PROBE_MTIME_1);
  return canary_extra(0) == 1 ? 0 : 1;
}
