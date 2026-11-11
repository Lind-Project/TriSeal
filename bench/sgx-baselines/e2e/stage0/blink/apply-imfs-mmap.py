import sys

root = sys.argv[1]

p = root + "/blink/map.c"
s = open(p).read()
helper = '''// Lind's IMFS and TriSeal cannot always map files, so copy the file into
// anonymous memory. Only safe when writes need not reach the file.
static void *MmapFileByCopy(void *addr, size_t length, int prot, int flags,
                            int fd, off_t offset) {
  void *res;
  ssize_t got;
  size_t done;
  if ((flags & MAP_SHARED) && (prot & PROT_WRITE)) return MAP_FAILED;
  res = VfsMmap(addr, length, PROT_READ | PROT_WRITE,
                (flags & ~MAP_SHARED) | MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
  if (res == MAP_FAILED) {
    LOGF("file copy: anonymous mmap of %zu bytes failed: %s", length,
         strerror(errno));
    return res;
  }
  for (done = 0; done < length; done += got) {
    got = pread(fd, (char *)res + done, length - done, offset + done);
    if (got == -1) {
      LOGF("file copy: pread of fd %d failed: %s", fd, strerror(errno));
      VfsMunmap(res, length);
      return MAP_FAILED;
    }
    if (!got) break;
  }
  return res;
}

static void *PortableMmap('''
old = "static void *PortableMmap("
assert s.count(old) == 1
s = s.replace(old, helper, 1)
old_tail = '''      errno = EEXIST;
    }
  }
#endif
  return res;
}
'''
new_tail = '''      errno = EEXIST;
    }
  }
#endif
  if (res == MAP_FAILED && fd != -1) {
    if (errno != ENOSYS && errno != EOPNOTSUPP && errno != ENODEV) {
      LOGF("mmap of fd %d failed: %s; copying instead", fd, strerror(errno));
    }
    res = MmapFileByCopy(addr, length, prot, flags, fd, offset);
  }
  return res;
}
'''
assert s.count(old_tail) == 1, s.count(old_tail)
s = s.replace(old_tail, new_tail, 1)
open(p, "w").write(s)

p = root + "/blink/loader.c"
s = open(p).read()
old = "img = VfsMmap(0, st.st_size, PROT_READ, MAP_SHARED, fd, 0);"
assert s.count(old) == 1
s = s.replace(old, 'img = Mmap(0, st.st_size, PROT_READ, MAP_SHARED, fd, 0, "loader");', 1)
open(p, "w").write(s)
print("patched")
