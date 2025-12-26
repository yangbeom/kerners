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
    *(.data*) 
    _edata = .;
  }
  .bss : { 
    _bss = .;
    *(.bss*) 
    _ebss = .;
  }
  
  _end = .;
}
PROVIDE(_stack_start = _end + 0x40000);
