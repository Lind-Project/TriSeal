# Usage:
#   make APP=forkexecv                      build + sign
#   make APP=forkexecv run                  run in SGX
#   make APP=forkexecv direct               run without SGX (gramine-direct)
#   make APP=microbench run ARGS="100000 20"
# Knobs (env): STATS=1 (needs a DEBUG-built Gramine; the apt package rejects it) EDMM=1 ENCLAVE_SIZE=256M MAX_THREADS=8 LOG=error

APP ?= hello
ARGS ?=
LOG ?= error
SRC := ../src
# exec targets that workloads start from automated_tests/
HELPERS := hello nop

.PHONY: all run direct clean
all: $(APP).manifest.sgx

app/$(APP): $(SRC)/$(APP).c | app/automated_tests
	gcc -O2 -o $@ $<

app/automated_tests/%: $(SRC)/%.c | app/automated_tests
	gcc -O2 -o $@ $<

app/automated_tests:
	mkdir -p $@

$(APP).manifest: app.manifest.template app/$(APP) $(HELPERS:%=app/automated_tests/%)
	gramine-manifest -Dapp=$(APP) -Dlog_level=$(LOG) $< $@

$(APP).manifest.sgx: $(APP).manifest
	gramine-sgx-sign --manifest $< --output $@ >/dev/null

run: $(APP).manifest.sgx
	gramine-sgx $(APP) $(ARGS)

direct: $(APP).manifest
	gramine-direct $(APP) $(ARGS)

clean:
	rm -rf app *.manifest *.manifest.sgx *.sig
