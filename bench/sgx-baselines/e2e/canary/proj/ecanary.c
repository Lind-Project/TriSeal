/* The environment canary: what the build sees of the machine it runs on.
 * env.h is written during the build by `probe env`; __FILE__ is the path the
 * compiler was given (an absolute one, as CMake passes). */
#include <stdio.h>
#include "env.h"

#define P(name) printf("%s=%s\n", #name, PROBE_##name)

int main(void) {
  printf("__FILE__=%s\n", __FILE__);
  P(ENV_COUNT); P(ENV_HASH);
  P(ENV_HOME); P(ENV_PATH); P(ENV_LANG); P(ENV_LC_ALL); P(ENV_TZ); P(ENV_USER);
  P(ENV_LOGNAME); P(ENV_PWD); P(ENV_SHELL); P(ENV_TMPDIR); P(ENV_SOURCE_DATE_EPOCH);
  P(UNAME_SYSNAME); P(UNAME_NODENAME); P(UNAME_RELEASE); P(UNAME_VERSION); P(UNAME_MACHINE);
  P(UID_GID); P(USER_NAME); P(HOSTNAME); P(DOMAINNAME); P(NPROCS); P(UMASK); P(CWD);
  P(READDIR); P(LOCALTIME); P(LOCALE); P(LOCALE_TIME); P(LOCALE_NUMBER); P(ADDRESSES);
  return 0;
}
