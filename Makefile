# Kerners OS - Build & Test Makefile

ARCH ?= aarch64
BUSYBOX ?=

.PHONY: test test-aarch64 test-riscv64 test-all busybox-smoke clean

# Default test target
test: test-$(ARCH)

test-aarch64:
	./scripts/run_tests.sh aarch64

test-riscv64:
	./scripts/run_tests.sh riscv64

test-all: test-aarch64 test-riscv64

busybox-smoke:
	./scripts/run_busybox_smoke.sh $(ARCH) "$(BUSYBOX)" 3 30

clean:
	cargo clean
	rm -f disk.img disk_test.img
