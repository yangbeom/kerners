# Kerners OS - Build & Test Makefile

ARCH ?= aarch64

.PHONY: test test-aarch64 test-riscv64 test-all clean

# Default test target
test: test-$(ARCH)

test-aarch64:
	./scripts/run_tests.sh aarch64

test-riscv64:
	./scripts/run_tests.sh riscv64

test-all: test-aarch64 test-riscv64

clean:
	cargo clean
	rm -f disk_test.img
