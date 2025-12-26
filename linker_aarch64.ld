ENTRY(_start);
SECTIONS
{
  /* QEMU virt: RAM starts at 0x40000000, kernel loaded at 0x40080000 */
  . = 0x40080000;
  
  _kernel_start = .;
  
  .text : {
    *(.text.boot)              /* Linux ARM64 부트 헤더가 맨 앞에 */
    *(.text.exception_vectors)
    *(.text*)
  }
  .rodata : { *(.rodata*) }
  .data : { *(.data*) }
  
  .bss : { 
    _bss_start = .;
    *(.bss*) 
    *(COMMON)
    _bss_end = .;
  }
  
  . = ALIGN(4096);
  _end = .;
}

/* 스택: 1MB */
PROVIDE(_stack_start = _end + 0x100000);
