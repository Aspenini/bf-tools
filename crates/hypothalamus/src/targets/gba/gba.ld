ENTRY(_start)

MEMORY
{
  ROM   (rx)  : ORIGIN = 0x08000000, LENGTH = 32M
  EWRAM (rwx) : ORIGIN = 0x02000000, LENGTH = 256K
  IWRAM (rwx) : ORIGIN = 0x03000000, LENGTH = 32K
}

SECTIONS
{
  . = ORIGIN(ROM);

  .gba_header :
  {
    KEEP(*(.gba_header))
  } > ROM

  .text :
  {
    KEEP(*(.text.start*))
    *(.text*)
    *(.rodata*)
    *(.glue_7)
    *(.glue_7t)
  } > ROM

  . = ALIGN(4);

  __data_load = LOADADDR(.data);
  .data : ALIGN(4)
  {
    __data_start = .;
    *(.data*)
    . = ALIGN(4);
    __data_end = .;
  } > EWRAM AT > ROM

  .bss (NOLOAD) : ALIGN(4)
  {
    __bss_start = .;
    *(.bss*)
    *(COMMON)
    . = ALIGN(4);
    __bss_end = .;
  } > EWRAM

  /DISCARD/ :
  {
    *(.comment*)
    *(.note*)
    *(.ARM.exidx*)
    *(.ARM.extab*)
    *(.ARM.attributes)
  }
}
