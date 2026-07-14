    .section .text.start, "ax", %progbits
    .arm
    .global _start
    .type _start, %function
_start:
    ldr sp, =__stack_top

    ldr r0, =__data_load
    ldr r1, =__data_start
    ldr r2, =__data_end
1:
    cmp r1, r2
    bhs 2f
    ldr r3, [r0], #4
    str r3, [r1], #4
    b 1b

2:
    ldr r0, =__bss_start
    ldr r1, =__bss_end
    mov r2, #0
3:
    cmp r0, r1
    bhs 4f
    str r2, [r0], #4
    b 3b

4:
    bl runtime_main

5:
    b 5b
    .size _start, . - _start
