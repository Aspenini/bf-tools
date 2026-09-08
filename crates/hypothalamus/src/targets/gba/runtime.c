/*
 * Game Boy Advance console runtime.
 *
 * Mode 3 gives a 240x160 framebuffer of 16-bit pixels. This divides it into a
 * 40x20 grid of 6x8 character cells and drives it as a text console: the whole
 * printable ASCII range renders from a 5x7 font, the control characters a
 * program is likely to emit behave the way a terminal would, and reaching the
 * bottom of the screen scrolls it rather than wiping it.
 */

typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;

extern void hypothalamus_bf_entry(void) __asm__("__HYPOTHALAMUS_ENTRY_SYMBOL__");

#define REG_DISPCNT (*(volatile u16 *)0x04000000)
#define REG_VCOUNT  (*(volatile u16 *)0x04000006)
#define VRAM        ((volatile u16 *)0x06000000)

/*
 * DMA channel 3 is the general-purpose one. An immediate transfer halts the
 * CPU until it finishes, so a copy is done by the next instruction - which is
 * what makes scrolling a 75 KB framebuffer once per line affordable.
 */
#define REG_DMA3SAD (*(volatile u32 *)0x040000D4)
#define REG_DMA3DAD (*(volatile u32 *)0x040000D8)
#define REG_DMA3CNT (*(volatile u32 *)0x040000DC)
#define DMA_ENABLE    0x80000000u
#define DMA_32BIT     0x04000000u
#define DMA_SRC_FIXED 0x01000000u

#define SCREEN_W 240u
#define SCREEN_H 160u
#define CELL_W 6u
#define CELL_H 8u
#define COLS (SCREEN_W / CELL_W)
#define ROWS (SCREEN_H / CELL_H)
#define GLYPH_W 5u
#define GLYPH_H 7u
#define TAB_WIDTH 4u
#define COLOR_BG 0x0000u
#define COLOR_FG 0x7FFFu

#define FONT_FIRST 0x20u
#define FONT_LAST 0x7Eu

/*
 * The printable ASCII range, one entry per character from FONT_FIRST. Each
 * entry is seven scanlines of a five-pixel-wide glyph, bit 4 leftmost.
 */
static const u8 FONT[FONT_LAST - FONT_FIRST + 1u][GLYPH_H] = {
    {0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00}, /* space */
    {0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04}, /* bang */
    {0x0A, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00}, /* double quote */
    {0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A}, /* hash */
    {0x04, 0x0F, 0x14, 0x0E, 0x05, 0x1E, 0x04}, /* dollar */
    {0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03}, /* percent */
    {0x08, 0x14, 0x14, 0x08, 0x15, 0x12, 0x0D}, /* ampersand */
    {0x04, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00}, /* apostrophe */
    {0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02}, /* open paren */
    {0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08}, /* close paren */
    {0x00, 0x04, 0x15, 0x0E, 0x15, 0x04, 0x00}, /* asterisk */
    {0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00}, /* plus */
    {0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08}, /* comma */
    {0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00}, /* minus */
    {0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C}, /* period */
    {0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10}, /* slash */
    {0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E}, /* 0 */
    {0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E}, /* 1 */
    {0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F}, /* 2 */
    {0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E}, /* 3 */
    {0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02}, /* 4 */
    {0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E}, /* 5 */
    {0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E}, /* 6 */
    {0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08}, /* 7 */
    {0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E}, /* 8 */
    {0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C}, /* 9 */
    {0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00}, /* colon */
    {0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x04, 0x08}, /* semicolon */
    {0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02}, /* less than */
    {0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00}, /* equals */
    {0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08}, /* greater than */
    {0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04}, /* question */
    {0x0E, 0x11, 0x01, 0x0D, 0x15, 0x15, 0x0E}, /* at */
    {0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11}, /* A */
    {0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E}, /* B */
    {0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E}, /* C */
    {0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C}, /* D */
    {0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F}, /* E */
    {0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10}, /* F */
    {0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F}, /* G */
    {0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11}, /* H */
    {0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E}, /* I */
    {0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C}, /* J */
    {0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11}, /* K */
    {0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F}, /* L */
    {0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11}, /* M */
    {0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11}, /* N */
    {0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E}, /* O */
    {0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10}, /* P */
    {0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D}, /* Q */
    {0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11}, /* R */
    {0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E}, /* S */
    {0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04}, /* T */
    {0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E}, /* U */
    {0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04}, /* V */
    {0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A}, /* W */
    {0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11}, /* X */
    {0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04}, /* Y */
    {0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F}, /* Z */
    {0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E}, /* open bracket */
    {0x10, 0x10, 0x08, 0x04, 0x02, 0x01, 0x01}, /* backslash */
    {0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E}, /* close bracket */
    {0x04, 0x0A, 0x11, 0x00, 0x00, 0x00, 0x00}, /* caret */
    {0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F}, /* underscore */
    {0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00}, /* backtick */
    {0x00, 0x00, 0x0E, 0x01, 0x0F, 0x11, 0x0F}, /* a */
    {0x10, 0x10, 0x1E, 0x11, 0x11, 0x11, 0x1E}, /* b */
    {0x00, 0x00, 0x0E, 0x11, 0x10, 0x11, 0x0E}, /* c */
    {0x01, 0x01, 0x0F, 0x11, 0x11, 0x11, 0x0F}, /* d */
    {0x00, 0x00, 0x0E, 0x11, 0x1F, 0x10, 0x0E}, /* e */
    {0x06, 0x09, 0x08, 0x1C, 0x08, 0x08, 0x08}, /* f */
    {0x00, 0x0F, 0x11, 0x11, 0x0F, 0x01, 0x0E}, /* g */
    {0x10, 0x10, 0x16, 0x19, 0x11, 0x11, 0x11}, /* h */
    {0x04, 0x00, 0x0C, 0x04, 0x04, 0x04, 0x0E}, /* i */
    {0x02, 0x00, 0x06, 0x02, 0x02, 0x12, 0x0C}, /* j */
    {0x10, 0x10, 0x12, 0x14, 0x18, 0x14, 0x12}, /* k */
    {0x0C, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E}, /* l */
    {0x00, 0x00, 0x1A, 0x15, 0x15, 0x15, 0x15}, /* m */
    {0x00, 0x00, 0x16, 0x19, 0x11, 0x11, 0x11}, /* n */
    {0x00, 0x00, 0x0E, 0x11, 0x11, 0x11, 0x0E}, /* o */
    {0x00, 0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10}, /* p */
    {0x00, 0x0F, 0x11, 0x11, 0x0F, 0x01, 0x01}, /* q */
    {0x00, 0x00, 0x16, 0x19, 0x10, 0x10, 0x10}, /* r */
    {0x00, 0x00, 0x0F, 0x10, 0x0E, 0x01, 0x1E}, /* s */
    {0x08, 0x08, 0x1C, 0x08, 0x08, 0x09, 0x06}, /* t */
    {0x00, 0x00, 0x11, 0x11, 0x11, 0x13, 0x0D}, /* u */
    {0x00, 0x00, 0x11, 0x11, 0x11, 0x0A, 0x04}, /* v */
    {0x00, 0x00, 0x11, 0x11, 0x15, 0x15, 0x0A}, /* w */
    {0x00, 0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11}, /* x */
    {0x00, 0x11, 0x11, 0x11, 0x0F, 0x01, 0x0E}, /* y */
    {0x00, 0x00, 0x1F, 0x02, 0x04, 0x08, 0x1F}, /* z */
    {0x02, 0x04, 0x04, 0x08, 0x04, 0x04, 0x02}, /* open brace */
    {0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04}, /* pipe */
    {0x08, 0x04, 0x04, 0x02, 0x04, 0x04, 0x08}, /* close brace */
    {0x00, 0x00, 0x08, 0x15, 0x02, 0x00, 0x00}, /* tilde */
};

/* Anything outside the font - a raw byte a program wrote - draws as a box. */
static const u8 FONT_FALLBACK[GLYPH_H] = {0x1F, 0x11, 0x15, 0x11, 0x15, 0x11, 0x1F};

static u32 cursor_x;
static u32 cursor_y;

/* Volatile so the value reaches memory before DMA is pointed at it. */
static volatile u32 fill_pattern;

static void wait_vblank(void) {
    while (REG_VCOUNT >= 160u) {}
    while (REG_VCOUNT < 160u) {}
}

static void dma3_copy(volatile u16 *dst, const volatile u16 *src, u32 words) {
    REG_DMA3SAD = (u32)src;
    REG_DMA3DAD = (u32)dst;
    REG_DMA3CNT = words | DMA_32BIT | DMA_ENABLE;
}

static void dma3_fill(volatile u16 *dst, u16 value, u32 words) {
    fill_pattern = ((u32)value << 16) | (u32)value;
    REG_DMA3SAD = (u32)&fill_pattern;
    REG_DMA3DAD = (u32)dst;
    REG_DMA3CNT = words | DMA_SRC_FIXED | DMA_32BIT | DMA_ENABLE;
}

static void clear_screen(void) {
    dma3_fill(VRAM, COLOR_BG, (SCREEN_W * SCREEN_H) / 2u);
    cursor_x = 0;
    cursor_y = 0;
}

/* Move every row up by one cell and blank the one that opens at the bottom. */
static void scroll_up(void) {
    const u32 line = CELL_H * SCREEN_W;

    dma3_copy(VRAM, VRAM + line, ((ROWS - 1u) * line) / 2u);
    dma3_fill(VRAM + (ROWS - 1u) * line, COLOR_BG, line / 2u);
}

static void newline(void) {
    cursor_x = 0;
    if (cursor_y + 1u < ROWS) {
        cursor_y++;
    } else {
        scroll_up();
    }
}

static void draw_char(u8 ch) {
    const u8 *glyph = FONT_FALLBACK;
    if (ch >= FONT_FIRST && ch <= FONT_LAST) {
        glyph = FONT[ch - FONT_FIRST];
    }

    volatile u16 *cell = VRAM + cursor_y * CELL_H * SCREEN_W + cursor_x * CELL_W;
    for (u32 row = 0; row < CELL_H; row++) {
        u8 bits = row < GLYPH_H ? glyph[row] : 0u;
        for (u32 col = 0; col < CELL_W; col++) {
            u16 color = COLOR_BG;
            if (col < GLYPH_W && (bits & (u8)(0x10u >> col)) != 0u) {
                color = COLOR_FG;
            }
            cell[col] = color;
        }
        cell += SCREEN_W;
    }
}

void hypothalamus_bf_putchar(u8 byte) __asm__("__HYPOTHALAMUS_PUTCHAR_SYMBOL__");
void hypothalamus_bf_putchar(u8 byte) {
    switch (byte) {
    case '\n':
        newline();
        return;
    case '\r':
        cursor_x = 0;
        return;
    case '\t':
        /* Step to the next tab stop, wrapping the way any other output does. */
        do {
            if (cursor_x >= COLS) {
                newline();
            }
            cursor_x++;
        } while (cursor_x % TAB_WIDTH != 0u);
        return;
    case '\f':
        clear_screen();
        return;
    case '\b':
        if (cursor_x > 0u) {
            cursor_x--;
        }
        return;
    case 0x07: /* bell: there is no speaker set up to ring */
        return;
    default:
        break;
    }

    /*
     * Wrap only once another cell is actually needed, so a line that exactly
     * fills the width does not also produce an empty one.
     */
    if (cursor_x >= COLS) {
        newline();
    }

    draw_char(byte);
    cursor_x++;
}

int hypothalamus_bf_getchar(void) __asm__("__HYPOTHALAMUS_GETCHAR_SYMBOL__");
int hypothalamus_bf_getchar(void) {
    return -1;
}

void runtime_main(void) {
    REG_DISPCNT = 0x0403u; /* Mode 3, BG2 enabled */
    clear_screen();
    hypothalamus_bf_entry();
    for (;;) {
        wait_vblank();
    }
}

/*
 * ---- freestanding runtime support ----
 *
 * At -O2 LLVM recognises the run of stores that clears the tape and rewrites it
 * as a memset, which on ARM lowers to the EABI helpers below. A freestanding
 * link has no libc to take them from, so the runtime supplies them itself.
 *
 * Watch the argument order: __aeabi_memset takes its length before its fill
 * byte, the opposite way round from memset.
 *
 * The runtime is compiled -ffreestanding -fno-builtin, which is what stops the
 * compiler from spotting the byte loops here and rewriting them into calls to
 * the very functions they implement.
 */

typedef unsigned int usize;
typedef u32 __attribute__((may_alias)) u32_alias;

void *memset(void *dest, int value, usize count) {
    u8 *out = (u8 *)dest;
    u8 byte = (u8)value;
    u32 word;

    while (count != 0u && ((u32)out & 3u) != 0u) {
        *out++ = byte;
        count--;
    }

    word = ((u32)byte << 24) | ((u32)byte << 16) | ((u32)byte << 8) | (u32)byte;
    while (count >= 4u) {
        *(u32_alias *)out = word;
        out += 4;
        count -= 4u;
    }

    while (count != 0u) {
        *out++ = byte;
        count--;
    }

    return dest;
}

void *memcpy(void *dest, const void *src, usize count) {
    u8 *out = (u8 *)dest;
    const u8 *in = (const u8 *)src;

    if ((((u32)out | (u32)in) & 3u) == 0u) {
        while (count >= 4u) {
            *(u32_alias *)out = *(const u32_alias *)in;
            out += 4;
            in += 4;
            count -= 4u;
        }
    }

    while (count != 0u) {
        *out++ = *in++;
        count--;
    }

    return dest;
}

void *memmove(void *dest, const void *src, usize count) {
    u8 *out = (u8 *)dest;
    const u8 *in = (const u8 *)src;

    if (out == in || count == 0u) {
        return dest;
    }
    if (out < in) {
        return memcpy(dest, src, count);
    }

    out += count;
    in += count;
    while (count != 0u) {
        *--out = *--in;
        count--;
    }

    return dest;
}

/* The 4 and 8 suffixed helpers only promise more alignment, which the generic
 * implementations already take advantage of when they see it. */
void __aeabi_memcpy(void *dest, const void *src, usize count) { memcpy(dest, src, count); }
void __aeabi_memcpy4(void *dest, const void *src, usize count) { memcpy(dest, src, count); }
void __aeabi_memcpy8(void *dest, const void *src, usize count) { memcpy(dest, src, count); }

void __aeabi_memmove(void *dest, const void *src, usize count) { memmove(dest, src, count); }
void __aeabi_memmove4(void *dest, const void *src, usize count) { memmove(dest, src, count); }
void __aeabi_memmove8(void *dest, const void *src, usize count) { memmove(dest, src, count); }

void __aeabi_memset(void *dest, usize count, int value) { memset(dest, value, count); }
void __aeabi_memset4(void *dest, usize count, int value) { memset(dest, value, count); }
void __aeabi_memset8(void *dest, usize count, int value) { memset(dest, value, count); }

void __aeabi_memclr(void *dest, usize count) { memset(dest, 0, count); }
void __aeabi_memclr4(void *dest, usize count) { memset(dest, 0, count); }
void __aeabi_memclr8(void *dest, usize count) { memset(dest, 0, count); }
