ENTRY(_start)

MEMORY
{
  MAINRAM (rwx) : ORIGIN = 0x02000000, LENGTH = 4M
}

SECTIONS
{
  . = ORIGIN(MAINRAM);

  .text :
  {
    KEEP(*(.text.start*))
    *(.text*)
    *(.rodata*)
    *(.glue_7)
    *(.glue_7t)
  } > MAINRAM

  . = ALIGN(4);

  __data_load = LOADADDR(.data);
  .data : ALIGN(4)
  {
    __data_start = .;
    *(.data*)
    . = ALIGN(4);
    __data_end = .;
  } > MAINRAM

  .bss (NOLOAD) : ALIGN(4)
  {
    __bss_start = .;
    *(.bss*)
    *(COMMON)
    . = ALIGN(4);
    __bss_end = .;
  } > MAINRAM

  . = ALIGN(8);
  __stack_top = ORIGIN(MAINRAM) + LENGTH(MAINRAM);

  /DISCARD/ :
  {
    *(.comment*)
    *(.note*)
    *(.ARM.exidx*)
    *(.ARM.extab*)
    *(.ARM.attributes)
  }
}
