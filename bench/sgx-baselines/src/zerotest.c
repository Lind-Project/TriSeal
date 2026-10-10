/*
 * Fresh anonymous memory must read as zero: glibc's calloc and allocators
 * such as gcc's ggc rely on it. Checks memory handed back after munmap,
 * after shrinking and regrowing the heap with sbrk, and from MAP_FIXED over
 * a dirty mapping. Prints one "@zero <case> ok|DIRTY <n> bytes" line each.
 */
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#define SIZE (1 << 20)

static void report(const char *name, const unsigned char *p, size_t n)
{
  size_t dirty = 0;
  for (size_t i = 0; i < n; i++)
    dirty += p[i] != 0;
  printf("@zero %s %s %zu\n", name, dirty ? "DIRTY" : "ok", dirty);
  fflush(stdout);
}

int main(void)
{
  unsigned char *p = mmap(NULL, SIZE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
  memset(p, 0xab, SIZE);
  munmap(p, SIZE);
  unsigned char *q = mmap(NULL, SIZE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
  printf("@zero mmap-reuse same_address=%d\n", p == q);
  report("mmap-after-munmap", q, SIZE);

  memset(q, 0xcd, SIZE);
  unsigned char *r = mmap(q, SIZE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
  report("mmap-fixed-over-dirty", r, SIZE);

  unsigned char *a = sbrk(SIZE);
  memset(a, 0xef, SIZE);
  sbrk(-SIZE);
  unsigned char *b = sbrk(SIZE);
  printf("@zero sbrk-reuse same_address=%d\n", a == b);
  report("sbrk-after-shrink", b, SIZE);
  return 0;
}
