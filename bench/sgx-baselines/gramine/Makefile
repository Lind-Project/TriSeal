# Usage:
#   make APP=forkexecv                      build + sign
#   make APP=forkexecv run                  run in SGX
#   make APP=forkexecv direct               run without SGX (gramine-direct)
# Knobs (env): STATS=1 (needs a DEBUG-built Gramine; the apt package rejects it) EDMM=1 ENCLAVE_SIZE=256M MAX_THREADS=8 LOG=error

APP ?= hello
LOG ?= error
SRC := ../src

.PHONY: all run direct clean
all: $(APP).manifest.sgx

app/$(APP) app/automated_tests/hello: | app/automated_tests
	gcc -O2 -o app/$(APP) $(SRC)/$(APP).c
	gcc -O2 -o app/automated_tests/hello $(SRC)/hello.c

app/automated_tests:
	mkdir -p $@

$(APP).manifest: app.manifest.template app/$(APP) app/automated_tests/hello
	gramine-manifest -Dapp=$(APP) -Dlog_level=$(LOG) $< $@

$(APP).manifest.sgx: $(APP).manifest
	gramine-sgx-sign --manifest $< --output $@ >/dev/null

run: $(APP).manifest.sgx
	gramine-sgx $(APP)

direct: $(APP).manifest
	gramine-direct $(APP)

clean:
	rm -rf app *.manifest *.manifest.sgx *.sig
