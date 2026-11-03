# The image canary/vary.sh runs builds in when a variation needs another uid
# or host name: the host's Ubuntu with what the drivers take from the host
# besides the bench tree: bc (timing), python3 and openssl (verify-attest.py),
# file, binutils (the native build's as and ar) and gcc (the TriSeal drivers
# list the headers to pack with gcc -M; the same gcc 13 as the host's).
FROM ubuntu:latest
RUN apt-get update && apt-get install -y --no-install-recommends bc python3 openssl file binutils gcc libc6-dev && rm -rf /var/lib/apt/lists/*
