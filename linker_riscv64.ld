ENTRY(_start);
SECTIONS
{
  . = 0x80000000;
  
  .text : { 
    _text = .;
    *(.text.boot)
    *(.text.trap_vector)
    *(.text*) 
    _etext = .;
  }
  .rodata : { 
    _rodata = .;
    *(.rodata*) 
    _erodata = .;
  }
  .data : { 
    _data = .;
    *(.sdata .sdata.*)
    *(.data .data.*)
    _edata = .;
  }
  . = ALIGN(16);
  PROVIDE(__global_pointer$ = . + 0x800);
  .bss : { 
    _bss = .;
    *(.sbss .sbss.*)
    *(.bss .bss.*)
    *(COMMON)
    _ebss = .;
  }
  
  _end = .;
}
PROVIDE(_stack_start = _end + 0x40000);
