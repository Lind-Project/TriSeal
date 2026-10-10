/* cachetest helper: record that the B variant ran. */
#include <fcntl.h>
#include <unistd.h>

int main(void)
{
  int fd = open("/tmp/cachetest.out", O_CREAT | O_TRUNC | O_WRONLY, 0644);
  write(fd, "BBBB\n", 5);
  close(fd);
  return 0;
}
