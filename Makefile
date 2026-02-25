# Kerners OS - Build & Test Makefile

ARCH ?= aarch64
BUSYBOX ?=

.PHONY: test test-aarch64 test-riscv64 test-all-kernel test-user-aarch64 test-user-riscv64 test-user test-all busybox-smoke clean

# Default test target
test: test-$(ARCH)

test-aarch64:
	./scripts/run_tests.sh aarch64

test-riscv64:
	./scripts/run_tests.sh riscv64

test-all-kernel: test-aarch64 test-riscv64

test-user-aarch64:
	./scripts/verify_phase15_3_cdyn.sh aarch64

test-user-riscv64:
	./scripts/verify_phase15_3_cdyn.sh riscv64

test-user: test-user-aarch64 test-user-riscv64

test-all: test-all-kernel test-user

busybox-smoke:
	./scripts/run_busybox_smoke.sh $(ARCH) "$(BUSYBOX)" 3 30

clean:
	cargo clean
	rm -f disk.img disk_test.img
