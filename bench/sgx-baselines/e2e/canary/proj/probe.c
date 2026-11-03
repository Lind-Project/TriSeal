/* probe: a build-time tool that writes what it reads from the system into a
 * C header, as a code generator that stamps its build would. The canary
 * project (Makefile) runs it during the build and compiles the header into
 * its program, so every value it reads ends up in the build's outputs.
 *
 *   probe time <out.h> [file...]   the clocks a build can read (time,
 *                                  gettimeofday, CLOCK_REALTIME, _MONOTONIC)
 *                                  and the mtime of each file
 *   probe env <out.h> <dir> [class...]
 *                                  what reprotest varies, by class: vars
 *                                  (the environment), uname, ids (uid, gid,
 *                                  user name), host (host and domain name),
 *                                  cpus, umask, cwd, readdir (the order
 *                                  readdir returns <dir> in), tz, locale,
 *                                  addr (addresses: ASLR); default all.
 *                                  A class left out defines its macros as
 *                                  "(not probed)", so the header is the same
 *                                  shape either way.
 *
 * Every value is a string macro PROBE_<NAME>; canary.c prints them.
 * Plain POSIX, so the same source builds natively and for lind.
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <locale.h>
#include <pwd.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/utsname.h>
#include <time.h>
#include <unistd.h>

extern char **environ;
static FILE *out;

static void def(const char *name, const char *value) {
  fprintf(out, "#define PROBE_%s \"", name);
  for (const char *p = value ? value : "(null)"; *p; p++) {
    if (*p == '"' || *p == '\\') fputc('\\', out);
    if (*p == '\n') fputs("\\n", out);
    else fputc(*p, out);
  }
  fputs("\"\n", out);
}

static void deff(const char *name, const char *fmt, long long a, long long b) {
  char buf[64];
  snprintf(buf, sizeof buf, fmt, a, b);
  def(name, buf);
}

static char **classes;
static int nclasses;

static int want(const char *cls) {
  if (nclasses == 0) return 1;
  for (int i = 0; i < nclasses; i++)
    if (!strcmp(classes[i], cls)) return 1;
  return 0;
}

static void skip(const char *names) {  /* space-separated macro names */
  char buf[256], *save, *t;
  snprintf(buf, sizeof buf, "%s", names);
  for (t = strtok_r(buf, " ", &save); t; t = strtok_r(NULL, " ", &save)) def(t, "(not probed)");
}

static int cmp(const void *a, const void *b) {
  return strcmp(*(char *const *)a, *(char *const *)b);
}

static void probe_time(int n, char **files) {
  struct timeval tv;
  struct timespec ts;
  deff("TIME", "%lld", (long long)time(NULL), 0);
  gettimeofday(&tv, NULL);
  deff("GETTIMEOFDAY", "%lld.%06lld", (long long)tv.tv_sec, (long long)tv.tv_usec);
  clock_gettime(CLOCK_REALTIME, &ts);
  deff("REALTIME", "%lld.%09lld", (long long)ts.tv_sec, (long long)ts.tv_nsec);
  clock_gettime(CLOCK_MONOTONIC, &ts);
  deff("MONOTONIC", "%lld.%09lld", (long long)ts.tv_sec, (long long)ts.tv_nsec);
  for (int i = 0; i < n; i++) {
    struct stat st;
    char name[64];
    snprintf(name, sizeof name, "MTIME_%d", i);
    if (stat(files[i], &st) == 0)
      deff(name, "%lld.%09lld", (long long)st.st_mtim.tv_sec, (long long)st.st_mtim.tv_nsec);
    else
      def(name, "stat failed");
  }
}

static void probe_env(const char *dir) {
  char buf[4096];
  /* The environment: its size, a hash over every entry (in sorted order, so
   * that only the contents count), and the variables reprotest varies. */
  static const char *vars[] = {"HOME", "PATH", "LANG", "LC_ALL", "TZ", "USER", "LOGNAME",
                               "PWD", "SHELL", "TMPDIR", "SOURCE_DATE_EPOCH", NULL};
  if (want("vars")) {
    int n = 0;
    while (environ && environ[n]) n++;
    char **env = malloc((n + 1) * sizeof *env);
    memcpy(env, environ, n * sizeof *env);
    qsort(env, n, sizeof *env, cmp);
    unsigned long long h = 1469598103934665603ULL; /* FNV-1a */
    for (int i = 0; i < n; i++)
      for (const char *p = env[i]; ; p++) {
        h = (h ^ (unsigned char)*p) * 1099511628211ULL;
        if (!*p) break;
      }
    free(env);
    deff("ENV_COUNT", "%lld", n, 0);
    snprintf(buf, sizeof buf, "%016llx", h);
    def("ENV_HASH", buf);
    for (int i = 0; vars[i]; i++) {
      snprintf(buf, sizeof buf, "ENV_%s", vars[i]);
      def(buf, getenv(vars[i]) ? getenv(vars[i]) : "(unset)");
    }
  } else {
    skip("ENV_COUNT ENV_HASH");
    for (int i = 0; vars[i]; i++) {
      snprintf(buf, sizeof buf, "ENV_%s", vars[i]);
      def(buf, "(not probed)");
    }
  }

  if (want("uname")) {
    struct utsname u;
    if (uname(&u) == 0) {
      def("UNAME_SYSNAME", u.sysname);
      def("UNAME_NODENAME", u.nodename);
      def("UNAME_RELEASE", u.release);
      def("UNAME_VERSION", u.version);
      def("UNAME_MACHINE", u.machine);
    } else {
      skip("UNAME_SYSNAME UNAME_NODENAME UNAME_RELEASE UNAME_VERSION UNAME_MACHINE");
    }
  } else {
    skip("UNAME_SYSNAME UNAME_NODENAME UNAME_RELEASE UNAME_VERSION UNAME_MACHINE");
  }

  if (want("ids")) {
    deff("UID_GID", "%lld:%lld", (long long)getuid(), (long long)getgid());
    struct passwd *pw = getpwuid(getuid());
    def("USER_NAME", pw ? pw->pw_name : "(no passwd entry)");
  } else {
    skip("UID_GID USER_NAME");
  }
  if (want("host")) {
    def("HOSTNAME", gethostname(buf, sizeof buf) == 0 ? buf : "failed");
    def("DOMAINNAME", getdomainname(buf, sizeof buf) == 0 ? buf : "failed");
  } else {
    skip("HOSTNAME DOMAINNAME");
  }
  if (want("cpus"))
    deff("NPROCS", "%lld/%lld", sysconf(_SC_NPROCESSORS_ONLN), sysconf(_SC_NPROCESSORS_CONF));
  else
    skip("NPROCS");
  if (want("umask")) {
    mode_t m = umask(0);
    umask(m);
    deff("UMASK", "%04llo", (long long)m, 0);
  } else {
    skip("UMASK");
  }
  if (want("cwd"))
    def("CWD", getcwd(buf, sizeof buf) ? buf : "failed");
  else
    skip("CWD");

  /* readdir order of <dir>, as a build that globs or lists a directory
   * without sorting it would see it */
  if (want("readdir")) {
    buf[0] = 0;
    DIR *d = opendir(dir);
    if (d) {
      struct dirent *e;
      while ((e = readdir(d)))
        if (e->d_name[0] != '.' && strlen(buf) + strlen(e->d_name) + 2 < sizeof buf) {
          strcat(buf, e->d_name);
          strcat(buf, " ");
        }
      closedir(d);
    } else {
      strcpy(buf, "opendir failed");
    }
    def("READDIR", buf);
  } else {
    skip("READDIR");
  }

  /* A fixed instant in the local time zone, and in the locale's format. */
  time_t t = 1700000000;
  struct tm tm;
  if (want("tz")) {
    localtime_r(&t, &tm);
    strftime(buf, sizeof buf, "%Y-%m-%d %H:%M:%S %Z", &tm);
    def("LOCALTIME", buf);
  } else {
    skip("LOCALTIME");
  }
  if (want("locale")) {
    const char *loc = setlocale(LC_ALL, "");
    def("LOCALE", loc ? loc : "(setlocale failed)");
    gmtime_r(&t, &tm);
    strftime(buf, sizeof buf, "%c", &tm);
    def("LOCALE_TIME", buf);
    snprintf(buf, sizeof buf, "%.2f", 1234.5);
    def("LOCALE_NUMBER", buf);
  } else {
    skip("LOCALE LOCALE_TIME LOCALE_NUMBER");
  }

  /* Addresses, as a build that hashes pointers or prints them would leak:
   * the stack, the heap and the code. */
  if (want("addr")) {
    int local;
    void *heap = malloc(64);
    snprintf(buf, sizeof buf, "%p %p %p", (void *)&local, heap, (void *)&probe_env);
    free(heap);
    def("ADDRESSES", buf);
  } else {
    skip("ADDRESSES");
  }
}

int main(int argc, char **argv) {
  if (argc < 3 || (strcmp(argv[1], "time") && strcmp(argv[1], "env")) ||
      (!strcmp(argv[1], "env") && argc < 4)) {
    fprintf(stderr, "usage: probe time <out.h> [file...] | probe env <out.h> <dir> [class...]\n");
    return 2;
  }
  out = fopen(argv[2], "w");
  if (!out) {
    perror(argv[2]);
    return 1;
  }
  fprintf(out, "/* written by probe %s during the build */\n", argv[1]);
  if (!strcmp(argv[1], "time"))
    probe_time(argc - 3, argv + 3);
  else {
    classes = argv + 4;
    nclasses = argc - 4;
    probe_env(argv[3]);
  }
  return fclose(out) == 0 ? 0 : 1;
}
