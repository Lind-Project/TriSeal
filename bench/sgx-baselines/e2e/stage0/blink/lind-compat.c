// Symbols Blink's syscall.c needs that lind's glibc does not provide.
#include <errno.h>
#include <sys/types.h>
#include <time.h>

int getgroups(int size, gid_t list[]) {
  errno = ENOSYS;
  return -1;
}

int clock_nanosleep(clockid_t clock, int flags, const struct timespec *req,
                    struct timespec *rem) {
  if (flags) return ENOSYS;
  return nanosleep(req, rem) ? errno : 0;
}
