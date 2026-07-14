typedef unsigned char u8;
typedef unsigned int u32;

extern void bf_main(void);

#define OUTPUT_CAPACITY 1024u

volatile u32 hypothalamus_nds_output_len;
volatile u8 hypothalamus_nds_output[OUTPUT_CAPACITY];

void bf_putchar(u8 byte) {
    u32 index = hypothalamus_nds_output_len;
    if (index < OUTPUT_CAPACITY) {
        hypothalamus_nds_output[index] = byte;
        hypothalamus_nds_output_len = index + 1u;
    }
}

int bf_getchar(void) {
    return -1;
}

void runtime_main(void) {
    hypothalamus_nds_output_len = 0;
    bf_main();
    for (;;) {}
}
