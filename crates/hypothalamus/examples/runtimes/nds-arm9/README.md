# Nintendo DS ARM9 Runtime Example

This directory contains a minimal ARM9 link example for Hypothalamus'
`nds-arm9` target. It produces an ARM9 ELF payload, not a complete `.nds`
image.

Build the Brainfuck payload object:

```sh
hypothalamus --target nds-arm9 examples/hello.bf -o /tmp/hello_arm9.o
```

Compile and link the example runtime:

```sh
clang --target=armv5te-none-eabi -mcpu=arm946e-s -marm \
  -x assembler-with-cpp -c examples/runtimes/nds-arm9/start.S \
  -o /tmp/nds_start.o

clang --target=armv5te-none-eabi -mcpu=arm946e-s -marm \
  -ffreestanding -fno-builtin -fno-unwind-tables \
  -fno-asynchronous-unwind-tables -Os -std=c99 \
  -c examples/runtimes/nds-arm9/runtime.c \
  -o /tmp/nds_runtime.o

ld.lld -m armelf -T examples/runtimes/nds-arm9/arm9.ld \
  /tmp/nds_start.o /tmp/nds_runtime.o /tmp/hello_arm9.o \
  -o /tmp/hello_arm9.elf
```

The runtime stores output bytes in `hypothalamus_nds_output` and returns EOF
from `bf_getchar`. A real DS program still needs an ARM7 companion, packaging,
boot metadata, and any display/input/IPC runtime layer you want to provide.
