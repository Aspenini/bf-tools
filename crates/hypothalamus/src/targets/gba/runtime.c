typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;

extern void hypothalamus_bf_entry(void) __asm__("__HYPOTHALAMUS_ENTRY_SYMBOL__");

#define REG_DISPCNT (*(volatile u16 *)0x04000000)
#define REG_VCOUNT  (*(volatile u16 *)0x04000006)
#define VRAM        ((volatile u16 *)0x06000000)
#define SCREEN_W 240u
#define SCREEN_H 160u
#define CELL_W 6u
#define CELL_H 8u
#define COLS 40u
#define ROWS 20u
#define COLOR_BG 0x0000u
#define COLOR_FG 0x7FFFu

static u32 cursor_x;
static u32 cursor_y;

static void wait_vblank(void) {
    while (REG_VCOUNT >= 160u) {}
    while (REG_VCOUNT < 160u) {}
}

static void clear_screen(void) {
    for (u32 i = 0; i < SCREEN_W * SCREEN_H; i++) {
        VRAM[i] = COLOR_BG;
    }
    cursor_x = 0;
    cursor_y = 0;
}

static u8 glyph_row(u8 ch, u32 row) {
    if (ch >= 'a' && ch <= 'z') {
        ch = (u8)(ch - ('a' - 'A'));
    }

    switch (ch) {
    case 'A': switch (row) { case 0: return 0x0E; case 1: return 0x11; case 2: return 0x11; case 3: return 0x1F; case 4: return 0x11; case 5: return 0x11; case 6: return 0x11; } break;
    case 'B': switch (row) { case 0: return 0x1E; case 1: return 0x11; case 2: return 0x11; case 3: return 0x1E; case 4: return 0x11; case 5: return 0x11; case 6: return 0x1E; } break;
    case 'C': switch (row) { case 0: return 0x0E; case 1: return 0x11; case 2: return 0x10; case 3: return 0x10; case 4: return 0x10; case 5: return 0x11; case 6: return 0x0E; } break;
    case 'D': switch (row) { case 0: return 0x1E; case 1: return 0x11; case 2: return 0x11; case 3: return 0x11; case 4: return 0x11; case 5: return 0x11; case 6: return 0x1E; } break;
    case 'E': switch (row) { case 0: return 0x1F; case 1: return 0x10; case 2: return 0x10; case 3: return 0x1E; case 4: return 0x10; case 5: return 0x10; case 6: return 0x1F; } break;
    case 'F': switch (row) { case 0: return 0x1F; case 1: return 0x10; case 2: return 0x10; case 3: return 0x1E; case 4: return 0x10; case 5: return 0x10; case 6: return 0x10; } break;
    case 'G': switch (row) { case 0: return 0x0E; case 1: return 0x11; case 2: return 0x10; case 3: return 0x17; case 4: return 0x11; case 5: return 0x11; case 6: return 0x0E; } break;
    case 'H': switch (row) { case 0: return 0x11; case 1: return 0x11; case 2: return 0x11; case 3: return 0x1F; case 4: return 0x11; case 5: return 0x11; case 6: return 0x11; } break;
    case 'I': switch (row) { case 0: return 0x0E; case 1: return 0x04; case 2: return 0x04; case 3: return 0x04; case 4: return 0x04; case 5: return 0x04; case 6: return 0x0E; } break;
    case 'J': switch (row) { case 0: return 0x01; case 1: return 0x01; case 2: return 0x01; case 3: return 0x01; case 4: return 0x11; case 5: return 0x11; case 6: return 0x0E; } break;
    case 'K': switch (row) { case 0: return 0x11; case 1: return 0x12; case 2: return 0x14; case 3: return 0x18; case 4: return 0x14; case 5: return 0x12; case 6: return 0x11; } break;
    case 'L': switch (row) { case 0: return 0x10; case 1: return 0x10; case 2: return 0x10; case 3: return 0x10; case 4: return 0x10; case 5: return 0x10; case 6: return 0x1F; } break;
    case 'M': switch (row) { case 0: return 0x11; case 1: return 0x1B; case 2: return 0x15; case 3: return 0x15; case 4: return 0x11; case 5: return 0x11; case 6: return 0x11; } break;
    case 'N': switch (row) { case 0: return 0x11; case 1: return 0x19; case 2: return 0x15; case 3: return 0x13; case 4: return 0x11; case 5: return 0x11; case 6: return 0x11; } break;
    case 'O': switch (row) { case 0: return 0x0E; case 1: return 0x11; case 2: return 0x11; case 3: return 0x11; case 4: return 0x11; case 5: return 0x11; case 6: return 0x0E; } break;
    case 'P': switch (row) { case 0: return 0x1E; case 1: return 0x11; case 2: return 0x11; case 3: return 0x1E; case 4: return 0x10; case 5: return 0x10; case 6: return 0x10; } break;
    case 'Q': switch (row) { case 0: return 0x0E; case 1: return 0x11; case 2: return 0x11; case 3: return 0x11; case 4: return 0x15; case 5: return 0x12; case 6: return 0x0D; } break;
    case 'R': switch (row) { case 0: return 0x1E; case 1: return 0x11; case 2: return 0x11; case 3: return 0x1E; case 4: return 0x14; case 5: return 0x12; case 6: return 0x11; } break;
    case 'S': switch (row) { case 0: return 0x0F; case 1: return 0x10; case 2: return 0x10; case 3: return 0x0E; case 4: return 0x01; case 5: return 0x01; case 6: return 0x1E; } break;
    case 'T': switch (row) { case 0: return 0x1F; case 1: return 0x04; case 2: return 0x04; case 3: return 0x04; case 4: return 0x04; case 5: return 0x04; case 6: return 0x04; } break;
    case 'U': switch (row) { case 0: return 0x11; case 1: return 0x11; case 2: return 0x11; case 3: return 0x11; case 4: return 0x11; case 5: return 0x11; case 6: return 0x0E; } break;
    case 'V': switch (row) { case 0: return 0x11; case 1: return 0x11; case 2: return 0x11; case 3: return 0x11; case 4: return 0x11; case 5: return 0x0A; case 6: return 0x04; } break;
    case 'W': switch (row) { case 0: return 0x11; case 1: return 0x11; case 2: return 0x11; case 3: return 0x15; case 4: return 0x15; case 5: return 0x15; case 6: return 0x0A; } break;
    case 'X': switch (row) { case 0: return 0x11; case 1: return 0x11; case 2: return 0x0A; case 3: return 0x04; case 4: return 0x0A; case 5: return 0x11; case 6: return 0x11; } break;
    case 'Y': switch (row) { case 0: return 0x11; case 1: return 0x11; case 2: return 0x0A; case 3: return 0x04; case 4: return 0x04; case 5: return 0x04; case 6: return 0x04; } break;
    case 'Z': switch (row) { case 0: return 0x1F; case 1: return 0x01; case 2: return 0x02; case 3: return 0x04; case 4: return 0x08; case 5: return 0x10; case 6: return 0x1F; } break;
    case '0': switch (row) { case 0: return 0x0E; case 1: return 0x11; case 2: return 0x13; case 3: return 0x15; case 4: return 0x19; case 5: return 0x11; case 6: return 0x0E; } break;
    case '1': switch (row) { case 0: return 0x04; case 1: return 0x0C; case 2: return 0x04; case 3: return 0x04; case 4: return 0x04; case 5: return 0x04; case 6: return 0x0E; } break;
    case '2': switch (row) { case 0: return 0x0E; case 1: return 0x11; case 2: return 0x01; case 3: return 0x02; case 4: return 0x04; case 5: return 0x08; case 6: return 0x1F; } break;
    case '3': switch (row) { case 0: return 0x1E; case 1: return 0x01; case 2: return 0x01; case 3: return 0x0E; case 4: return 0x01; case 5: return 0x01; case 6: return 0x1E; } break;
    case '4': switch (row) { case 0: return 0x02; case 1: return 0x06; case 2: return 0x0A; case 3: return 0x12; case 4: return 0x1F; case 5: return 0x02; case 6: return 0x02; } break;
    case '5': switch (row) { case 0: return 0x1F; case 1: return 0x10; case 2: return 0x10; case 3: return 0x1E; case 4: return 0x01; case 5: return 0x01; case 6: return 0x1E; } break;
    case '6': switch (row) { case 0: return 0x0E; case 1: return 0x10; case 2: return 0x10; case 3: return 0x1E; case 4: return 0x11; case 5: return 0x11; case 6: return 0x0E; } break;
    case '7': switch (row) { case 0: return 0x1F; case 1: return 0x01; case 2: return 0x02; case 3: return 0x04; case 4: return 0x08; case 5: return 0x08; case 6: return 0x08; } break;
    case '8': switch (row) { case 0: return 0x0E; case 1: return 0x11; case 2: return 0x11; case 3: return 0x0E; case 4: return 0x11; case 5: return 0x11; case 6: return 0x0E; } break;
    case '9': switch (row) { case 0: return 0x0E; case 1: return 0x11; case 2: return 0x11; case 3: return 0x0F; case 4: return 0x01; case 5: return 0x01; case 6: return 0x0E; } break;
    case '!': switch (row) { case 0: return 0x04; case 1: return 0x04; case 2: return 0x04; case 3: return 0x04; case 4: return 0x04; case 5: return 0x00; case 6: return 0x04; } break;
    case '?': switch (row) { case 0: return 0x0E; case 1: return 0x11; case 2: return 0x01; case 3: return 0x02; case 4: return 0x04; case 5: return 0x00; case 6: return 0x04; } break;
    case '.': switch (row) { case 5: return 0x00; case 6: return 0x04; } break;
    case ',': switch (row) { case 5: return 0x04; case 6: return 0x08; } break;
    case ':': switch (row) { case 1: return 0x04; case 5: return 0x04; } break;
    case '-': switch (row) { case 3: return 0x1F; } break;
    case '+': switch (row) { case 2: return 0x04; case 3: return 0x1F; case 4: return 0x04; } break;
    case '/': switch (row) { case 0: return 0x01; case 1: return 0x02; case 2: return 0x02; case 3: return 0x04; case 4: return 0x08; case 5: return 0x08; case 6: return 0x10; } break;
    case '<': switch (row) { case 1: return 0x02; case 2: return 0x04; case 3: return 0x08; case 4: return 0x04; case 5: return 0x02; } break;
    case '>': switch (row) { case 1: return 0x08; case 2: return 0x04; case 3: return 0x02; case 4: return 0x04; case 5: return 0x08; } break;
    case '[': switch (row) { case 0: return 0x0E; case 1: return 0x08; case 2: return 0x08; case 3: return 0x08; case 4: return 0x08; case 5: return 0x08; case 6: return 0x0E; } break;
    case ']': switch (row) { case 0: return 0x0E; case 1: return 0x02; case 2: return 0x02; case 3: return 0x02; case 4: return 0x02; case 5: return 0x02; case 6: return 0x0E; } break;
    case ' ': return 0x00;
    default: switch (row) { case 0: return 0x1F; case 1: return 0x11; case 2: return 0x15; case 3: return 0x11; case 4: return 0x15; case 5: return 0x11; case 6: return 0x1F; } break;
    }
    return 0x00;
}

static void newline(void) {
    cursor_x = 0;
    cursor_y++;
    if (cursor_y >= ROWS) {
        clear_screen();
    }
}

static void draw_char(u8 ch) {
    u32 px = cursor_x * CELL_W;
    u32 py = cursor_y * CELL_H;
    for (u32 row = 0; row < CELL_H; row++) {
        u8 bits = 0;
        if (row > 0 && row < 8u) {
            bits = glyph_row(ch, row - 1u);
        }
        for (u32 col = 0; col < CELL_W; col++) {
            u16 color = COLOR_BG;
            if (col < 5u && (bits & (u8)(1u << (4u - col))) != 0u) {
                color = COLOR_FG;
            }
            VRAM[(py + row) * SCREEN_W + px + col] = color;
        }
    }
}

void hypothalamus_bf_putchar(u8 byte) __asm__("__HYPOTHALAMUS_PUTCHAR_SYMBOL__");
void hypothalamus_bf_putchar(u8 byte) {
    if (byte == '\r') {
        return;
    }
    if (byte == '\n') {
        newline();
        return;
    }

    draw_char(byte);
    cursor_x++;
    if (cursor_x >= COLS) {
        newline();
    }
}

int hypothalamus_bf_getchar(void) __asm__("__HYPOTHALAMUS_GETCHAR_SYMBOL__");
int hypothalamus_bf_getchar(void) {
    return -1;
}

void runtime_main(void) {
    REG_DISPCNT = 0x0403u;
    clear_screen();
    hypothalamus_bf_entry();
    for (;;) {
        wait_vblank();
    }
}
