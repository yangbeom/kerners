# Kerners OS - Build & Test Makefile

ARCH ?= aarch64
BUSYBOX ?=

.PHONY: test test-kernel-aarch64 test-kernel-riscv64 test-aarch64 test-riscv64 test-all-kernel test-user-aarch64 test-user-riscv64 test-user test-all busybox-smoke clean

# Default test target
test: test-kernel-$(ARCH)

test-kernel-aarch64:
	./scripts/run_tests.sh aarch64

test-kernel-riscv64:
	./scripts/run_tests.sh riscv64

test-aarch64: test-kernel-aarch64

test-riscv64: test-kernel-riscv64

test-all-kernel: test-kernel-aarch64 test-kernel-riscv64

test-user-aarch64:
	./scripts/run_user_tests.sh aarch64

test-user-riscv64:
	./scripts/run_user_tests.sh riscv64

test-user: test-user-aarch64 test-user-riscv64

test-all: test-all-kernel test-user

busybox-smoke:
	./scripts/run_busybox_smoke.sh $(ARCH) "$(BUSYBOX)" 3 30

clean:
	cargo clean
	rm -f disk.img disk_test.img
