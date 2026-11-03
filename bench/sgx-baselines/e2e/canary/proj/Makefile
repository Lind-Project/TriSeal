# The canary project: a build that stamps itself with everything it can read
# of the time and of the machine, the way real builds do by accident.
#   make time   tcanary + libcanary.a: time(), gettimeofday, CLOCK_REALTIME,
#               CLOCK_MONOTONIC (probe time), __DATE__, __TIME__,
#               __TIMESTAMP__ (the compiler), the mtime of a preloaded
#               source and of a file written during the build, and ar's
#               timestamps (ar U: the symbol table's time, the members' mtimes)
#   make env    ecanary: what reprotest varies (probe env; ENV_CLASSES
#               limits it to some classes, see probe.c)
# Every recipe is one command with no shell syntax, so make runs it directly
# (a TriSeal keep has no shell).
CC = gcc
CFLAGS = -O2
AR = ar
ARFLAGS = rcU
RANLIB = ranlib -U
PROBE = probe
ENV_CLASSES =

time: tcanary libcanary.a
env: ecanary
.PHONY: time env

early.h:
	$(PROBE) time early.h
stamp.h: early.h tcanary.c
	$(PROBE) time stamp.h tcanary.c early.h
tcanary.o: tcanary.c stamp.h
	$(CC) $(CFLAGS) -c tcanary.c
extra.o: extra.c
	$(CC) $(CFLAGS) -c extra.c
tcanary: tcanary.o extra.o
	$(CC) -o tcanary tcanary.o extra.o
libcanary.a: tcanary.o extra.o
	$(AR) $(ARFLAGS) libcanary.a tcanary.o extra.o
	$(RANLIB) libcanary.a

env.h:
	$(PROBE) env env.h data $(ENV_CLASSES)
ecanary.o: ecanary.c env.h
	$(CC) $(CFLAGS) -c $(CURDIR)/ecanary.c -o ecanary.o
ecanary: ecanary.o
	$(CC) -o ecanary ecanary.o
