/* cachetest helper: record that the A variant ran. */
#include <fcntl.h>
#include <unistd.h>

int main(void)
{
  int fd = open("/tmp/cachetest.out", O_CREAT | O_TRUNC | O_WRONLY, 0644);
  write(fd, "AAAA\n", 5);
  close(fd);
  return 0;
}
