//! Brainfuck emitter with compile-time pointer tracking.
//!
//! Every Cranium value lives at a statically known tape address, so the builder
//! can always compute the `>`/`<` run needed to reach a cell. Loop helpers
//! guarantee the pointer is back where the loop started before `]` is emitted,
//! which keeps the static position exact across arbitrary nesting.
//!
//! Cells are the traditional wrapping bytes. Temporaries are handed out by a
//! bump allocator; [`Bf::scope`] releases everything a region allocated, and
//! every helper here leaves its temporaries at zero so reuse is safe.

/// A tape address, in cells from the start of the tape.
pub type Addr = i64;

/// Bits in one Brainfuck cell.
pub const BITS_PER_CELL: usize = 8;

/// A snapshot of both allocators, taken by [`Bf::watermark`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    scalar: Addr,
    array: Addr,
}

/// Emits Brainfuck while tracking the data pointer.
///
/// Two bump allocators share the tape. Scalars and temporaries come from the
/// low end and arrays from `array_base` upwards, because moving a value across
/// `d` cells costs `value * 2d` commands: putting a large array between two
/// scalars would tax every operation on them. Keeping the working set packed
/// together at the bottom is the single biggest thing the layout can do for
/// speed.
pub struct Bf {
    out: String,
    pos: Addr,
    next_free: Addr,
    peak: Addr,
    array_next: Addr,
    array_peak: Addr,
}

impl Default for Bf {
    fn default() -> Self {
        Self::new()
    }
}

impl Bf {
    /// Create an emitter whose arrays share the tape with everything else.
    ///
    /// Useful for a first pass that only needs to learn how much scalar space a
    /// program wants; see [`Bf::with_array_base`] for the real layout.
    pub fn new() -> Self {
        Self::with_array_base(0)
    }

    /// Create an emitter that places array regions at `array_base` and above.
    pub fn with_array_base(array_base: Addr) -> Self {
        Self {
            out: String::new(),
            pos: 0,
            next_free: 0,
            peak: 0,
            array_next: array_base,
            array_peak: array_base,
        }
    }

    /// The Brainfuck emitted so far.
    pub fn finish(self) -> String {
        self.out
    }

    /// Highest tape cell the program can touch.
    pub fn cells_used(&self) -> usize {
        self.peak.max(self.array_peak) as usize
    }

    /// Cells the scalar and temporary region needs.
    pub fn scalar_cells(&self) -> usize {
        self.peak as usize
    }

    /// Number of Brainfuck commands emitted so far.
    pub fn len(&self) -> usize {
        self.out.len()
    }

    /// True when nothing has been emitted.
    pub fn is_empty(&self) -> bool {
        self.out.is_empty()
    }

    /// Reserve `count` consecutive cells.
    ///
    /// The cells are not zeroed; callers that need a known value must write one.
    pub fn alloc(&mut self, count: usize) -> Addr {
        let base = self.next_free;
        self.next_free += count as Addr;
        self.peak = self.peak.max(self.next_free);
        base
    }

    /// Reserve `count` consecutive cells and zero them.
    pub fn alloc_zeroed(&mut self, count: usize) -> Addr {
        let base = self.alloc(count);
        for offset in 0..count as Addr {
            self.zero(base + offset);
        }
        base
    }

    /// Reserve `count` consecutive cells in the array region.
    ///
    /// The cells are not zeroed; callers that need a known value must write one.
    pub fn alloc_array(&mut self, count: usize) -> Addr {
        let base = self.array_next;
        self.array_next += count as Addr;
        self.array_peak = self.array_peak.max(self.array_next);
        base
    }

    /// Run `body` with a temporary allocation scope.
    ///
    /// Cells reserved inside `body` are released when it returns, so sibling
    /// regions reuse the same tape space.
    pub fn scope<T>(&mut self, body: impl FnOnce(&mut Self) -> T) -> T {
        let mark = self.watermark();
        let value = body(self);
        self.release_to(mark);
        value
    }

    /// The addresses both allocators would hand out next.
    pub fn watermark(&self) -> Mark {
        Mark {
            scalar: self.next_free,
            array: self.array_next,
        }
    }

    /// Reset both allocators to a previous [`Bf::watermark`].
    pub fn release_to(&mut self, mark: Mark) {
        self.next_free = mark.scalar;
        self.array_next = mark.array;
    }

    fn push(&mut self, ch: char) {
        self.out.push(ch);
    }

    /// Move the data pointer to `addr`.
    pub fn goto(&mut self, addr: Addr) {
        let delta = addr - self.pos;
        let (ch, count) = if delta >= 0 {
            ('>', delta)
        } else {
            ('<', -delta)
        };
        for _ in 0..count {
            self.push(ch);
        }
        self.pos = addr;
    }

    /// Add `delta` to the cell at `addr`, wrapping.
    pub fn add(&mut self, addr: Addr, delta: i32) {
        let delta = delta.rem_euclid(256);
        if delta == 0 {
            return;
        }
        self.goto(addr);
        // Reaching a target by subtraction is shorter past the halfway point.
        if delta <= 128 {
            for _ in 0..delta {
                self.push('+');
            }
        } else {
            for _ in 0..(256 - delta) {
                self.push('-');
            }
        }
    }

    /// Set the cell at `addr` to zero.
    pub fn zero(&mut self, addr: Addr) {
        self.goto(addr);
        self.out.push_str("[-]");
    }

    /// Set the cell at `addr` to `value`.
    pub fn set(&mut self, addr: Addr, value: u8) {
        self.zero(addr);
        self.add(addr, i32::from(value));
    }

    /// Write the cell at `addr` to standard output.
    pub fn write(&mut self, addr: Addr) {
        self.goto(addr);
        self.push('.');
    }

    /// Read one byte from standard input into `addr`.
    ///
    /// The cell is zeroed first so that end-of-file reads as `0` on every
    /// runtime, including ones that leave the cell untouched.
    pub fn read(&mut self, addr: Addr) {
        self.zero(addr);
        self.goto(addr);
        self.push(',');
    }

    /// Emit a literal byte to standard output using `scratch` as the source
    /// cell, leaving `scratch` holding `value`.
    pub fn write_literal(&mut self, scratch: Addr, value: u8) {
        self.set(scratch, value);
        self.write(scratch);
    }

    /// Emit `bytes` one at a time, reusing `scratch` and adjusting between
    /// characters instead of rebuilding each value from zero.
    pub fn write_bytes(&mut self, scratch: Addr, bytes: &[u8]) {
        let mut current: Option<u8> = None;
        for &byte in bytes {
            match current {
                Some(previous) => self.add(scratch, i32::from(byte) - i32::from(previous)),
                None => self.set(scratch, byte),
            }
            self.write(scratch);
            current = Some(byte);
        }
        if current.is_some() {
            self.zero(scratch);
        }
    }

    /// Append Brainfuck verbatim without updating the tracked position.
    ///
    /// This is the escape hatch for the array walks in [`crate::codegen`],
    /// where the pointer moves by a runtime amount. Callers must emit balanced
    /// brackets and follow up with [`Bf::set_pos`] naming the cell the pointer
    /// provably ends on.
    pub fn emit_raw(&mut self, code: &str) {
        debug_assert!(
            code.bytes()
                .all(|byte| matches!(byte, b'+' | b'-' | b'<' | b'>' | b'.' | b',' | b'[' | b']')),
            "raw brainfuck may only contain commands"
        );
        self.out.push_str(code);
    }

    /// Declare where the data pointer is after an [`Bf::emit_raw`] sequence.
    pub fn set_pos(&mut self, addr: Addr) {
        self.pos = addr;
    }

    /// Emit `[ body ]` anchored at `addr`.
    ///
    /// `body` may move the pointer freely; the builder returns to `addr` before
    /// closing the loop, so the loop re-tests the cell it started on.
    pub fn loop_at(&mut self, addr: Addr, body: impl FnOnce(&mut Self)) {
        self.goto(addr);
        self.push('[');
        body(self);
        self.goto(addr);
        self.push(']');
    }

    /// Emit `[` anchored at `addr`.
    ///
    /// Pair with [`Bf::close_loop`] on the same address. This is the open-coded
    /// form of [`Bf::loop_at`], for callers whose loop body needs `&mut self`
    /// on something other than the builder.
    pub fn open_loop(&mut self, addr: Addr) {
        self.goto(addr);
        self.push('[');
    }

    /// Emit `]` anchored at `addr`, closing an [`Bf::open_loop`].
    pub fn close_loop(&mut self, addr: Addr) {
        self.goto(addr);
        self.push(']');
    }

    /// Add the value in `src` to every cell in `dsts`, leaving `src` at zero.
    pub fn move_add(&mut self, src: Addr, dsts: &[Addr]) {
        if dsts.is_empty() {
            self.zero(src);
            return;
        }
        self.loop_at(src, |bf| {
            bf.add(src, -1);
            for &dst in dsts {
                bf.add(dst, 1);
            }
        });
    }

    /// Subtract the value in `src` from every cell in `dsts`, leaving `src` at
    /// zero.
    pub fn move_sub(&mut self, src: Addr, dsts: &[Addr]) {
        if dsts.is_empty() {
            self.zero(src);
            return;
        }
        self.loop_at(src, |bf| {
            bf.add(src, -1);
            for &dst in dsts {
                bf.add(dst, -1);
            }
        });
    }

    /// Add a copy of `src` to `dst` without disturbing `src`.
    pub fn add_copy(&mut self, src: Addr, dst: Addr) {
        self.scope(|bf| {
            let keep = bf.alloc_zeroed(1);
            bf.move_add(src, &[dst, keep]);
            bf.move_add(keep, &[src]);
        });
    }

    /// Subtract a copy of `src` from `dst` without disturbing `src`.
    pub fn sub_copy(&mut self, src: Addr, dst: Addr) {
        self.scope(|bf| {
            let keep = bf.alloc_zeroed(1);
            bf.loop_at(src, |bf| {
                bf.add(src, -1);
                bf.add(dst, -1);
                bf.add(keep, 1);
            });
            bf.move_add(keep, &[src]);
        });
    }

    /// Overwrite `dst` with a copy of `src`.
    pub fn copy(&mut self, src: Addr, dst: Addr) {
        if src == dst {
            return;
        }
        self.zero(dst);
        self.add_copy(src, dst);
    }

    /// Run `body` once if `cond` is nonzero, consuming `cond`.
    ///
    /// `body` must not leave `cond` nonzero; the helper clears it before the
    /// loop re-tests, so the block runs at most once.
    pub fn if_nonzero_consume(&mut self, cond: Addr, body: impl FnOnce(&mut Self)) {
        self.loop_at(cond, |bf| {
            body(bf);
            bf.zero(cond);
        });
    }

    /// Run `body` once if `cond` is nonzero, leaving `cond` unchanged.
    pub fn if_nonzero(&mut self, cond: Addr, body: impl FnOnce(&mut Self)) {
        self.scope(|bf| {
            let test = bf.alloc_zeroed(1);
            bf.add_copy(cond, test);
            bf.if_nonzero_consume(test, body);
        });
    }

    /// Run `then_body` if `cond` is nonzero and `else_body` otherwise,
    /// consuming `cond`.
    pub fn if_else_consume(
        &mut self,
        cond: Addr,
        then_body: impl FnOnce(&mut Self),
        else_body: impl FnOnce(&mut Self),
    ) {
        self.scope(|bf| {
            let otherwise = bf.alloc_zeroed(1);
            bf.set(otherwise, 1);
            bf.loop_at(cond, |bf| {
                then_body(bf);
                bf.zero(otherwise);
                bf.zero(cond);
            });
            bf.if_nonzero_consume(otherwise, else_body);
        });
    }

    /// Run `then_body` if `cond` is nonzero and `else_body` otherwise, leaving
    /// `cond` unchanged.
    pub fn if_else(
        &mut self,
        cond: Addr,
        then_body: impl FnOnce(&mut Self),
        else_body: impl FnOnce(&mut Self),
    ) {
        self.scope(|bf| {
            let test = bf.alloc_zeroed(1);
            bf.add_copy(cond, test);
            bf.if_else_consume(test, then_body, else_body);
        });
    }

    /// Run `body` if `cond` is zero, leaving `cond` unchanged.
    pub fn if_zero(&mut self, cond: Addr, body: impl FnOnce(&mut Self)) {
        self.if_else(cond, |_| {}, body);
    }

    /// Set `out` to `1` when `value` is zero and `0` otherwise.
    pub fn is_zero(&mut self, out: Addr, value: Addr) {
        self.set(out, 1);
        self.if_nonzero(value, |bf| bf.zero(out));
    }

    /// Set `out` to `1` when `value` is nonzero and `0` otherwise.
    pub fn is_nonzero(&mut self, out: Addr, value: Addr) {
        self.zero(out);
        self.if_nonzero(value, |bf| bf.set(out, 1));
    }

    /// Set `out` to `1` when the bytes at `lhs` and `rhs` are equal.
    ///
    /// Runs in time proportional to the operand values: the difference is
    /// formed by cancelling the two cells against each other.
    pub fn byte_eq(&mut self, out: Addr, lhs: Addr, rhs: Addr) {
        self.scope(|bf| {
            let diff = bf.alloc_zeroed(1);
            bf.add_copy(lhs, diff);
            bf.sub_copy(rhs, diff);
            bf.is_zero(out, diff);
            bf.zero(diff);
        });
    }

    /// Write the eight bits of `value` to `bits..bits + 8`, least significant
    /// first, leaving `value` unchanged.
    ///
    /// Repeated halving costs about twice the value, which is the cheapest way
    /// to get at a byte's structure on a tape whose only arithmetic is
    /// increment and decrement.
    pub fn byte_bits(&mut self, value: Addr, bits: Addr) {
        self.scope(|bf| {
            let rest = bf.alloc_zeroed(1);
            bf.add_copy(value, rest);
            for index in 0..BITS_PER_CELL as Addr {
                let half = bf.alloc_zeroed(1);
                bf.byte_divmod2(rest, half, bits + index);
                bf.move_add(half, &[rest]);
            }
            bf.zero(rest);
        });
    }

    /// Set `out` to `1` when the byte at `lhs` is less than the byte at `rhs`.
    ///
    /// Both operands are split into bits and compared from the top down. That
    /// costs about twice each operand's value, where cancelling the two cells
    /// against each other one step at a time would cost their product.
    pub fn byte_lt(&mut self, out: Addr, lhs: Addr, rhs: Addr) {
        self.scope(|bf| {
            let left = bf.alloc_zeroed(BITS_PER_CELL);
            let right = bf.alloc_zeroed(BITS_PER_CELL);
            bf.byte_bits(lhs, left);
            bf.byte_bits(rhs, right);

            bf.zero(out);
            let decided = bf.alloc_zeroed(1);
            for index in (0..BITS_PER_CELL as Addr).rev() {
                let left_bit = left + index;
                let right_bit = right + index;
                bf.if_zero(decided, |bf| {
                    // Every cell here holds 0 or 1, so these tests are cheap.
                    bf.if_else(
                        left_bit,
                        |bf| bf.if_zero(right_bit, |bf| bf.set(decided, 1)),
                        |bf| {
                            bf.if_nonzero(right_bit, |bf| {
                                bf.set(out, 1);
                                bf.set(decided, 1);
                            })
                        },
                    );
                });
            }

            bf.zero(decided);
            for index in 0..BITS_PER_CELL as Addr {
                bf.zero(left + index);
                bf.zero(right + index);
            }
        });
    }

    /// Halve the byte at `value`, writing the quotient to `quotient` and the
    /// remainder to `remainder`. `value` is consumed.
    pub fn byte_divmod2(&mut self, value: Addr, quotient: Addr, remainder: Addr) {
        self.zero(quotient);
        self.zero(remainder);
        self.loop_at(value, |bf| {
            bf.add(value, -1);
            bf.scope(|bf| {
                let was_odd = bf.alloc_zeroed(1);
                bf.set(was_odd, 1);
                // `remainder` only ever holds 0 or 1, so this runs at most once.
                bf.loop_at(remainder, |bf| {
                    bf.zero(remainder);
                    bf.add(quotient, 1);
                    bf.zero(was_odd);
                });
                bf.if_nonzero_consume(was_odd, |bf| bf.set(remainder, 1));
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lobe::{create_runtime, CellSize};

    fn run(src: &str) -> Vec<u8> {
        let mut runtime = create_runtime(src, CellSize::Bits8).expect("valid brainfuck");
        let mut input = std::io::Cursor::new(Vec::new());
        let mut output = Vec::new();
        runtime
            .run_with_io(&mut input, &mut output)
            .expect("program runs");
        output
    }

    #[test]
    fn tracks_the_pointer_across_loops() {
        let mut bf = Bf::new();
        let counter = bf.alloc_zeroed(1);
        let sink = bf.alloc_zeroed(1);
        bf.set(counter, 3);
        bf.loop_at(counter, |bf| {
            bf.add(counter, -1);
            bf.add(sink, 2);
        });
        bf.add(sink, 'A' as i32);
        bf.write(sink);
        assert_eq!(run(&bf.finish()), vec![b'A' + 6]);
    }

    #[test]
    fn copies_without_destroying_the_source() {
        let mut bf = Bf::new();
        let src = bf.alloc_zeroed(1);
        let dst = bf.alloc_zeroed(1);
        bf.set(src, b'x');
        bf.copy(src, dst);
        bf.write(src);
        bf.write(dst);
        assert_eq!(run(&bf.finish()), vec![b'x', b'x']);
    }

    #[test]
    fn compares_bytes() {
        for (lhs, rhs) in [(0_u8, 0_u8), (3, 9), (9, 3), (255, 254), (1, 255)] {
            let mut bf = Bf::new();
            let a = bf.alloc_zeroed(1);
            let b = bf.alloc_zeroed(1);
            let lt = bf.alloc_zeroed(1);
            let eq = bf.alloc_zeroed(1);
            bf.set(a, lhs);
            bf.set(b, rhs);
            bf.byte_lt(lt, a, b);
            bf.byte_eq(eq, a, b);
            bf.add(lt, b'0' as i32);
            bf.add(eq, b'0' as i32);
            bf.write(lt);
            bf.write(eq);
            let expected = vec![b'0' + u8::from(lhs < rhs), b'0' + u8::from(lhs == rhs)];
            assert_eq!(run(&bf.finish()), expected, "comparing {lhs} and {rhs}");
        }
    }

    #[test]
    fn halves_bytes() {
        for value in [0_u8, 1, 2, 7, 200, 255] {
            let mut bf = Bf::new();
            let cell = bf.alloc_zeroed(1);
            let quotient = bf.alloc_zeroed(1);
            let remainder = bf.alloc_zeroed(1);
            bf.set(cell, value);
            bf.byte_divmod2(cell, quotient, remainder);
            bf.write(quotient);
            bf.write(remainder);
            assert_eq!(run(&bf.finish()), vec![value / 2, value % 2]);
        }
    }

    #[test]
    fn scopes_reuse_tape_space() {
        let mut bf = Bf::new();
        bf.alloc(4);
        let before = bf.cells_used();
        bf.scope(|bf| {
            bf.alloc(16);
        });
        bf.alloc(4);
        assert_eq!(before, 4);
        assert_eq!(bf.cells_used(), 20);
        assert_eq!(bf.watermark(), bf.watermark());
        assert_eq!(bf.scalar_cells(), 20);
    }

    #[test]
    fn arrays_live_above_the_scalar_region() {
        let mut bf = Bf::with_array_base(64);
        let scalar = bf.alloc(2);
        let array = bf.alloc_array(100);
        assert_eq!(scalar, 0);
        assert_eq!(array, 64);
        assert_eq!(bf.scalar_cells(), 2);
        assert_eq!(bf.cells_used(), 164);
    }
}
