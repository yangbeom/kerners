# QEMU 실행 가이드

kerners 커널을 QEMU에서 빌드하고 실행하는 방법을 설명합니다.

## 빠른 시작

`run.sh` 스크립트로 DTB 생성과 커널 실행을 한 번에 처리합니다:

```bash
# aarch64 빌드 및 실행
./run.sh aarch64

# riscv64 빌드 및 실행
./run.sh riscv64

# 메모리 크기 지정 (기본값: 512MB)
./run.sh aarch64 1024

# 멀티코어 (SMP)
./run.sh aarch64 512 4    # 4코어
./run.sh riscv64 512 2    # 2코어

# DTB만 재생성
./run.sh aarch64 128 --dtb-only
```

## BusyBox `init` 부팅

prebuilt static BusyBox ELF가 있으면 `KERNERS_BUSYBOX` 환경변수로 `disk.img`를 자동 준비할 수 있습니다.
디스크 이미지는 `KERNERS_DISK_IMG` 환경변수로 경로를 통일해 사용할 수 있습니다
(`run.sh`, `scripts/run_tests.sh`, `scripts/prepare_test_disk.sh`, `scripts/prepare_user_disk.sh` 공통).

### BusyBox static 빌드

```bash
# kerners 워크스페이스에서 BusyBox static ELF 빌드
./scripts/build_busybox_static.sh aarch64 /Users/yangbeom/github/busybox

# 결과: target/user/aarch64/busybox
file target/user/aarch64/busybox
```

```bash
# BusyBox를 disk.img에 설치(/bin/busybox, /sbin/init, /bin/init, /bin/sh) 후 실행
KERNERS_BUSYBOX=/absolute/path/to/busybox ./run.sh aarch64

# 또는 스크립트만 단독 실행
./scripts/prepare_user_disk.sh aarch64 /absolute/path/to/busybox disk.img
```

커널은 부팅 후 기본 init 경로(`/sbin/init`, `/etc/init`, `/bin/init`, `/bin/sh`)를 먼저 탐색하고,
필요 시 `/mnt/*` 경로를 fallback으로 탐색합니다. 실행 실패 시 커널 셸로 복귀합니다.

### BusyBox init 스모크 테스트(로그 자동 수집)

```bash
# 3회 반복, 각 30초 타임아웃, logs/busybox-init-*.log 저장
./scripts/run_busybox_smoke.sh aarch64 /absolute/path/to/busybox 3 30

# 또는 Makefile 래퍼
make busybox-smoke ARCH=aarch64 BUSYBOX=/absolute/path/to/busybox
```

스모크 스크립트는 run별 로그와 summary 로그를 생성하고, 실패 시 원인을 1차 분류합니다.
(`ENOSYS`, `EFAULT`, `EXEC_FAIL`, `NO_INIT_FALLBACK`, `PANIC`, `TIMEOUT` 등)

`BUSYBOX_SMOKE_REQUIRE_COW=1`을 지정하면 `COW_FORK_TEST: PASS` 마커를 필수로 검사합니다.

### Phase 15-2 rcS 스모크 (`switch_root=fat32`)

아래 시나리오는 15-2 최소 경로(`cat` + `mkdir`)를 양 아키텍처에서 재현합니다.

```bash
ARCH=aarch64
BUSYBOX=target/user/${ARCH}/busybox
DISK=logs/phase15-2-${ARCH}.img

./scripts/prepare_user_disk.sh "$ARCH" "$BUSYBOX" "$DISK"
mmd -i "$DISK" ::/etc/init.d

cat >/tmp/rcS <<'EOF'
#!/bin/sh
echo PH15_2_BEGIN
cat /t15.txt
mkdir /
echo PH15_2_END
EOF
printf 'hello_phase15\n' >/tmp/t15.txt

mcopy -o -i "$DISK" /tmp/rcS ::/etc/init.d/rcS
mcopy -o -i "$DISK" /tmp/t15.txt ::/t15.txt

KERNERS_DISK_IMG="$DISK" \
KERNERS_BOOTARGS="kerners.root=fat32" \
./run.sh "$ARCH" 512 1
```

기대 로그 마커:
- `PH15_2_BEGIN`
- `hello_phase15`
- `PH15_2_END`
- `Kernel panic`, `Bad file descriptor` 미발생

## 수동 실행 방법

### 1. 빌드

```bash
# aarch64
cargo build --release --target aarch64-unknown-none-softfloat

# riscv64
cargo build --release --target riscv64gc-unknown-none-elf
```

### 2. DTB 파일 생성

DTB(Device Tree Blob)는 하드웨어 정보를 담고 있으며, QEMU 머신 설정에 따라 생성됩니다.
**메모리 크기(`-m`)를 변경하면 DTB를 다시 생성해야 합니다.**

```bash
# aarch64용 DTB 생성 (512MB)
qemu-system-aarch64 -machine virt,dumpdtb=virt_aarch64.dtb -cpu cortex-a57 -m 512M

# riscv64용 DTB 생성 (512MB)
qemu-system-riscv64 -machine virt,dumpdtb=virt_riscv64.dtb -m 512M
```

### 3. QEMU 실행

#### aarch64

```bash
qemu-system-aarch64 \
  -machine virt \
  -cpu cortex-a57 \
  -m 512M \
  -nographic \
  -kernel target/aarch64-unknown-none-softfloat/release/kerners \
  -device loader,file=virt_aarch64.dtb,addr=0x48000000,force-raw=on
```

#### riscv64

```bash
qemu-system-riscv64 \
  -machine virt \
  -m 512M \
  -nographic \
  -bios none \
  -kernel target/riscv64gc-unknown-none-elf/release/kerners \
  -device loader,file=virt_riscv64.dtb,addr=0x88000000,force-raw=on
```

### 4. QEMU 종료

`Ctrl+A`를 누른 후 `X` 키

## DTB 로드 주소

DTB는 **RAM 끝에서 2MB 전**에 배치됩니다. `run.sh`가 메모리 크기에 따라 자동 계산합니다:

```
DTB_ADDR = RAM_START + (MEMORY_MB * 1MB) - 2MB
```

### 512MB RAM 예시

| 아키텍처 | RAM 시작 | RAM 끝 | DTB 주소 |
|---------|----------|--------|----------|
| aarch64 | 0x40000000 | 0x60000000 | 0x5FE00000 |
| riscv64 | 0x80000000 | 0xA0000000 | 0x9FE00000 |

### 1GB RAM 예시

| 아키텍처 | RAM 시작 | RAM 끝 | DTB 주소 |
|---------|----------|--------|----------|
| aarch64 | 0x40000000 | 0x80000000 | 0x7FE00000 |
| riscv64 | 0x80000000 | 0xC0000000 | 0xBFE00000 |

## 문제 해결

### DTB 파싱 실패
- DTB 파일이 생성되었는지 확인: `ls -la virt_*.dtb`
- QEMU 실행 시 메모리 크기와 DTB 생성 시 메모리 크기가 일치하는지 확인
- `-device loader` 주소가 RAM 범위 내에 있는지 확인

### ROM regions overlapping 에러
- DTB 로드 주소가 커널과 겹치지 않도록 설정
- aarch64: 커널은 `0x40080000`에 로드되므로 DTB는 그 이후에 배치

### 아무 출력도 없음
- `-nographic` 옵션이 있는지 확인
- 커널 바이너리가 올바르게 빌드되었는지 확인: `file target/.../kerners`
