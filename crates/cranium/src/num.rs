//! Multi-cell unsigned arithmetic built on the [`Bf`] byte primitives.
//!
//! A Cranium number of width `w` occupies `w` consecutive cells in
//! little-endian order, so `byte` is one cell and `int` is two. Every routine
//! here is unrolled at compile time over the width, which keeps the emitted
//! Brainfuck free of any runtime notion of "how wide is this value".
//!
//! Costs are worth knowing when writing Cranium:
//!
//! - equality is linear in the operand values;
//! - ordering (`<`, `<=`, `>`, `>=`) cancels the operands against each other,
//!   so it is quadratic in the smaller byte of each cell pair;
//! - multiplication and division on `int` use shift-and-add, so they are
//!   bounded by the bit width rather than the operand values.

use crate::bf::{Addr, Bf};

/// Which bitwise operation [`Bf::num_bitwise`] should apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitKind {
    /// Bitwise and.
    And,
    /// Bitwise or.
    Or,
    /// Bitwise exclusive or.
    Xor,
}

/// Decimal place values used when printing, widest first.
fn decimal_places(width: usize) -> &'static [u64] {
    match width {
        1 => &[100, 10, 1],
        _ => &[10_000, 1_000, 100, 10, 1],
    }
}

impl Bf {
    /// Zero all `width` cells of the number at `addr`.
    pub fn num_zero(&mut self, addr: Addr, width: usize) {
        for offset in 0..width as Addr {
            self.zero(addr + offset);
        }
    }

    /// Store the little-endian bytes of `value` into the number at `addr`.
    pub fn num_set(&mut self, addr: Addr, width: usize, value: u64) {
        for offset in 0..width {
            let byte = (value >> (8 * offset)) as u8;
            self.set(addr + offset as Addr, byte);
        }
    }

    /// Overwrite the number at `dst` with the one at `src`.
    pub fn num_copy(&mut self, src: Addr, dst: Addr, width: usize) {
        if src == dst {
            return;
        }
        for offset in 0..width as Addr {
            self.copy(src + offset, dst + offset);
        }
    }

    /// Copy `src` into `dst`, zero-extending or truncating between widths.
    pub fn num_convert(&mut self, src: Addr, src_width: usize, dst: Addr, dst_width: usize) {
        for offset in 0..dst_width as Addr {
            if (offset as usize) < src_width {
                self.copy(src + offset, dst + offset);
            } else {
                self.zero(dst + offset);
            }
        }
    }

    /// Set `out` to `1` when any cell of the number at `addr` is nonzero.
    pub fn num_is_nonzero(&mut self, out: Addr, addr: Addr, width: usize) {
        self.zero(out);
        for offset in 0..width as Addr {
            let cell = addr + offset;
            self.if_nonzero(cell, |bf| bf.set(out, 1));
        }
    }

    /// Set `out` to `1` when every cell of the number at `addr` is zero.
    pub fn num_is_zero(&mut self, out: Addr, addr: Addr, width: usize) {
        self.set(out, 1);
        for offset in 0..width as Addr {
            let cell = addr + offset;
            self.if_nonzero(cell, |bf| bf.zero(out));
        }
    }

    /// Add the number at `src` into `dst`, leaving `src` unchanged.
    ///
    /// Overflow past the top cell is discarded, matching the wrapping
    /// arithmetic of the underlying tape.
    pub fn num_add_assign(&mut self, dst: Addr, src: Addr, width: usize) {
        self.scope(|bf| {
            let carry = bf.alloc_zeroed(1);
            for index in 0..width {
                let dst_cell = dst + index as Addr;
                let src_cell = src + index as Addr;
                let is_last = index + 1 == width;

                bf.scope(|bf| {
                    let addend = bf.alloc_zeroed(1);
                    bf.add_copy(src_cell, addend);

                    if is_last {
                        bf.move_add(addend, &[dst_cell]);
                        bf.move_add(carry, &[dst_cell]);
                        return;
                    }

                    let carry_in = bf.alloc_zeroed(1);
                    bf.move_add(carry, &[carry_in]);

                    bf.add_copy(addend, dst_cell);
                    // The sum wrapped exactly when it landed below the addend.
                    bf.scope(|bf| {
                        let wrapped = bf.alloc_zeroed(1);
                        bf.byte_lt(wrapped, dst_cell, addend);
                        bf.move_add(wrapped, &[carry]);
                    });

                    bf.if_nonzero_consume(carry_in, |bf| {
                        bf.add(dst_cell, 1);
                        bf.scope(|bf| {
                            let wrapped = bf.alloc_zeroed(1);
                            bf.is_zero(wrapped, dst_cell);
                            bf.move_add(wrapped, &[carry]);
                        });
                    });

                    bf.zero(addend);
                });
            }
            bf.zero(carry);
        });
    }

    /// Subtract the number at `src` from `dst`, leaving `src` unchanged.
    ///
    /// Borrowing past the top cell is discarded, so the result wraps.
    pub fn num_sub_assign(&mut self, dst: Addr, src: Addr, width: usize) {
        self.scope(|bf| {
            let borrow = bf.alloc_zeroed(1);
            for index in 0..width {
                let dst_cell = dst + index as Addr;
                let src_cell = src + index as Addr;
                let is_last = index + 1 == width;

                bf.scope(|bf| {
                    let subtrahend = bf.alloc_zeroed(1);
                    bf.add_copy(src_cell, subtrahend);

                    if is_last {
                        bf.move_sub(subtrahend, &[dst_cell]);
                        bf.move_sub(borrow, &[dst_cell]);
                        return;
                    }

                    let borrow_in = bf.alloc_zeroed(1);
                    bf.move_add(borrow, &[borrow_in]);

                    // The borrow depends on the value before the subtraction.
                    bf.scope(|bf| {
                        let wrapped = bf.alloc_zeroed(1);
                        bf.byte_lt(wrapped, dst_cell, subtrahend);
                        bf.move_add(wrapped, &[borrow]);
                    });
                    bf.sub_copy(subtrahend, dst_cell);

                    bf.if_nonzero_consume(borrow_in, |bf| {
                        bf.scope(|bf| {
                            let wrapped = bf.alloc_zeroed(1);
                            bf.is_zero(wrapped, dst_cell);
                            bf.move_add(wrapped, &[borrow]);
                        });
                        bf.add(dst_cell, -1);
                    });

                    bf.zero(subtrahend);
                });
            }
            bf.zero(borrow);
        });
    }

    /// Set `out` to `1` when the numbers at `lhs` and `rhs` are equal.
    pub fn num_eq(&mut self, out: Addr, lhs: Addr, rhs: Addr, width: usize) {
        self.set(out, 1);
        for index in 0..width as Addr {
            self.scope(|bf| {
                let same = bf.alloc_zeroed(1);
                bf.byte_eq(same, lhs + index, rhs + index);
                bf.if_zero(same, |bf| bf.zero(out));
                bf.zero(same);
            });
        }
    }

    /// Set `out` to `1` when the number at `lhs` is less than the one at `rhs`.
    ///
    /// Cells are compared most significant first, and the first difference
    /// decides the result.
    pub fn num_lt(&mut self, out: Addr, lhs: Addr, rhs: Addr, width: usize) {
        self.scope(|bf| {
            let decided = bf.alloc_zeroed(1);
            bf.zero(out);

            for index in (0..width as Addr).rev() {
                let left = lhs + index;
                let right = rhs + index;
                bf.if_zero(decided, |bf| {
                    bf.scope(|bf| {
                        let less = bf.alloc_zeroed(1);
                        bf.byte_lt(less, left, right);
                        bf.if_else_consume(
                            less,
                            |bf| {
                                bf.set(out, 1);
                                bf.set(decided, 1);
                            },
                            |bf| {
                                bf.scope(|bf| {
                                    let same = bf.alloc_zeroed(1);
                                    bf.byte_eq(same, left, right);
                                    bf.if_zero(same, |bf| bf.set(decided, 1));
                                    bf.zero(same);
                                });
                            },
                        );
                    });
                });
            }

            bf.zero(decided);
        });
    }

    /// Set `out` to `1` when `lhs` is greater than or equal to `rhs`.
    pub fn num_ge(&mut self, out: Addr, lhs: Addr, rhs: Addr, width: usize) {
        self.scope(|bf| {
            let less = bf.alloc_zeroed(1);
            bf.num_lt(less, lhs, rhs, width);
            bf.is_zero(out, less);
            bf.zero(less);
        });
    }

    /// Shift the number at `addr` left by one bit, writing the bit that fell
    /// off the top into `carry_out`.
    pub fn num_shl1(&mut self, addr: Addr, width: usize, carry_out: Addr) {
        self.scope(|bf| {
            let carry = bf.alloc_zeroed(1);
            for index in 0..width as Addr {
                let cell = addr + index;
                bf.scope(|bf| {
                    let original = bf.alloc_zeroed(1);
                    bf.add_copy(cell, original);
                    // Doubling wraps exactly when the top bit was set.
                    bf.add_copy(original, cell);
                    let next_carry = bf.alloc_zeroed(1);
                    bf.byte_lt(next_carry, cell, original);
                    // The doubled cell is even, so folding in the carry cannot wrap.
                    bf.move_add(carry, &[cell]);
                    bf.move_add(next_carry, &[carry]);
                    bf.zero(original);
                });
            }
            bf.zero(carry_out);
            bf.move_add(carry, &[carry_out]);
        });
    }

    /// Shift the number at `addr` right by one bit, writing the bit that fell
    /// off the bottom into `bit_out`.
    pub fn num_shr1(&mut self, addr: Addr, width: usize, bit_out: Addr) {
        self.scope(|bf| {
            let carry = bf.alloc_zeroed(1);
            for index in (0..width as Addr).rev() {
                let cell = addr + index;
                bf.scope(|bf| {
                    let quotient = bf.alloc_zeroed(1);
                    let remainder = bf.alloc_zeroed(1);
                    bf.byte_divmod2(cell, quotient, remainder);
                    bf.move_add(quotient, &[cell]);
                    bf.if_nonzero_consume(carry, |bf| bf.add(cell, 128));
                    bf.move_add(remainder, &[carry]);
                });
            }
            bf.zero(bit_out);
            bf.move_add(carry, &[bit_out]);
        });
    }

    /// Shift the number at `addr` left by a constant number of bits.
    pub fn num_shl_const(&mut self, addr: Addr, width: usize, amount: usize) {
        if amount >= width * 8 {
            self.num_zero(addr, width);
            return;
        }
        self.scope(|bf| {
            let discard = bf.alloc_zeroed(1);
            for _ in 0..amount {
                bf.num_shl1(addr, width, discard);
            }
            bf.zero(discard);
        });
    }

    /// Shift the number at `addr` right by a constant number of bits.
    pub fn num_shr_const(&mut self, addr: Addr, width: usize, amount: usize) {
        if amount >= width * 8 {
            self.num_zero(addr, width);
            return;
        }
        self.scope(|bf| {
            let discard = bf.alloc_zeroed(1);
            for _ in 0..amount {
                bf.num_shr1(addr, width, discard);
            }
            bf.zero(discard);
        });
    }

    /// Shift `dst` by the runtime amount in `count`, in the given direction.
    ///
    /// The loop runs once per unit of `count`, so large shift counts cost
    /// proportionally; constant shifts are unrolled by
    /// [`Bf::num_shl_const`] and [`Bf::num_shr_const`] instead.
    pub fn num_shift_dynamic(
        &mut self,
        dst: Addr,
        width: usize,
        count: Addr,
        count_width: usize,
        left: bool,
    ) {
        self.scope(|bf| {
            let remaining = bf.alloc_zeroed(count_width);
            bf.num_copy(count, remaining, count_width);
            let running = bf.alloc_zeroed(1);
            let discard = bf.alloc_zeroed(1);
            bf.num_is_nonzero(running, remaining, count_width);
            bf.loop_at(running, |bf| {
                bf.scope(|bf| {
                    let one = bf.alloc_zeroed(count_width);
                    bf.num_set(one, count_width, 1);
                    bf.num_sub_assign(remaining, one, count_width);
                    bf.num_zero(one, count_width);
                });
                if left {
                    bf.num_shl1(dst, width, discard);
                } else {
                    bf.num_shr1(dst, width, discard);
                }
                bf.num_is_nonzero(running, remaining, count_width);
            });
            bf.zero(discard);
            bf.num_zero(remaining, count_width);
        });
    }

    /// Store `lhs * rhs` in `out`.
    ///
    /// One-cell values use repeated addition, which keeps the emitted program
    /// small. Wider values use shift-and-add so the cost stays bounded by the
    /// bit width.
    pub fn num_mul(&mut self, out: Addr, lhs: Addr, rhs: Addr, width: usize) {
        self.num_zero(out, width);
        if width == 1 {
            self.scope(|bf| {
                let counter = bf.alloc_zeroed(1);
                bf.add_copy(rhs, counter);
                bf.loop_at(counter, |bf| {
                    bf.add(counter, -1);
                    bf.add_copy(lhs, out);
                });
            });
            return;
        }

        self.scope(|bf| {
            let shifted = bf.alloc_zeroed(width);
            let multiplier = bf.alloc_zeroed(width);
            let bit = bf.alloc_zeroed(1);
            let discard = bf.alloc_zeroed(1);
            bf.num_copy(lhs, shifted, width);
            bf.num_copy(rhs, multiplier, width);

            for _ in 0..(width * 8) {
                bf.num_shr1(multiplier, width, bit);
                bf.if_nonzero_consume(bit, |bf| bf.num_add_assign(out, shifted, width));
                bf.num_shl1(shifted, width, discard);
            }

            bf.zero(discard);
            bf.num_zero(shifted, width);
            bf.num_zero(multiplier, width);
        });
    }

    /// Store `src * factor` in `out` for a factor known at compile time.
    ///
    /// Only the set bits of `factor` cost anything, so `x * 10` is two
    /// additions and three doublings rather than a full multiply.
    pub fn num_mul_const(&mut self, out: Addr, src: Addr, width: usize, factor: u64) {
        self.num_zero(out, width);
        let bits = width * 8;
        let factor = factor & ((1_u128 << bits) - 1) as u64;
        if factor == 0 {
            return;
        }
        let highest = (u64::BITS - 1 - factor.leading_zeros()) as usize;

        self.scope(|bf| {
            let shifted = bf.alloc_zeroed(width);
            let discard = bf.alloc_zeroed(1);
            bf.num_copy(src, shifted, width);

            for bit in 0..=highest.min(bits - 1) {
                if factor & (1 << bit) != 0 {
                    bf.num_add_assign(out, shifted, width);
                }
                if bit < highest {
                    bf.num_shl1(shifted, width, discard);
                }
            }

            bf.zero(discard);
            bf.num_zero(shifted, width);
        });
    }

    /// Store `lhs / rhs` in `quotient` and `lhs % rhs` in `remainder`.
    ///
    /// Division by zero is defined rather than undefined: the quotient
    /// saturates to the widest representable value.
    pub fn num_divmod(
        &mut self,
        quotient: Addr,
        remainder: Addr,
        lhs: Addr,
        rhs: Addr,
        width: usize,
    ) {
        if width == 1 {
            self.divmod_byte(quotient, remainder, lhs, rhs);
            return;
        }

        self.num_zero(quotient, width);
        self.num_zero(remainder, width);
        self.scope(|bf| {
            let dividend = bf.alloc_zeroed(width);
            bf.num_copy(lhs, dividend, width);
            let top_bit = bf.alloc_zeroed(1);
            let rem_carry = bf.alloc_zeroed(1);
            let discard = bf.alloc_zeroed(1);

            for _ in 0..(width * 8) {
                bf.num_shl1(dividend, width, top_bit);
                bf.num_shl1(remainder, width, rem_carry);
                bf.move_add(top_bit, &[remainder]);
                bf.num_shl1(quotient, width, discard);

                bf.scope(|bf| {
                    let subtract = bf.alloc_zeroed(1);
                    bf.num_ge(subtract, remainder, rhs, width);
                    // A carry out of the remainder means it already exceeds the
                    // divisor, whatever the truncated cells say.
                    bf.if_nonzero_consume(rem_carry, |bf| bf.set(subtract, 1));
                    bf.if_nonzero_consume(subtract, |bf| {
                        bf.num_sub_assign(remainder, rhs, width);
                        bf.add(quotient, 1);
                    });
                });
            }

            bf.zero(discard);
            bf.num_zero(dividend, width);
        });
    }

    fn divmod_byte(&mut self, quotient: Addr, remainder: Addr, lhs: Addr, rhs: Addr) {
        self.zero(quotient);
        self.zero(remainder);
        self.scope(|bf| {
            let divisor_ok = bf.alloc_zeroed(1);
            bf.is_nonzero(divisor_ok, rhs);
            bf.if_else_consume(
                divisor_ok,
                |bf| {
                    bf.add_copy(lhs, remainder);
                    bf.scope(|bf| {
                        let keep_going = bf.alloc_zeroed(1);
                        bf.num_ge(keep_going, remainder, rhs, 1);
                        bf.loop_at(keep_going, |bf| {
                            bf.sub_copy(rhs, remainder);
                            bf.add(quotient, 1);
                            bf.num_ge(keep_going, remainder, rhs, 1);
                        });
                    });
                },
                |bf| bf.set(quotient, 255),
            );
        });
    }

    /// Store the bitwise combination of `lhs` and `rhs` in `out`.
    pub fn num_bitwise(&mut self, out: Addr, lhs: Addr, rhs: Addr, width: usize, kind: BitKind) {
        let bit_count = width * 8;
        self.scope(|bf| {
            let left = bf.alloc_zeroed(width);
            let right = bf.alloc_zeroed(width);
            let bits = bf.alloc_zeroed(bit_count);
            bf.num_copy(lhs, left, width);
            bf.num_copy(rhs, right, width);

            for index in 0..bit_count as Addr {
                bf.scope(|bf| {
                    let left_bit = bf.alloc_zeroed(1);
                    let right_bit = bf.alloc_zeroed(1);
                    bf.num_shr1(left, width, left_bit);
                    bf.num_shr1(right, width, right_bit);
                    let slot = bits + index;
                    match kind {
                        BitKind::And => {
                            bf.if_nonzero_consume(left_bit, |bf| {
                                bf.if_nonzero_consume(right_bit, |bf| bf.set(slot, 1));
                            });
                        }
                        BitKind::Or => {
                            bf.if_nonzero_consume(left_bit, |bf| bf.set(slot, 1));
                            bf.if_nonzero_consume(right_bit, |bf| bf.set(slot, 1));
                        }
                        BitKind::Xor => {
                            bf.move_add(left_bit, &[slot]);
                            bf.if_nonzero_consume(right_bit, |bf| {
                                bf.if_else(slot, |bf| bf.zero(slot), |bf| bf.set(slot, 1));
                            });
                        }
                    }
                    bf.zero(left_bit);
                    bf.zero(right_bit);
                });
            }

            bf.num_zero(out, width);
            bf.scope(|bf| {
                let discard = bf.alloc_zeroed(1);
                for index in (0..bit_count as Addr).rev() {
                    bf.num_shl1(out, width, discard);
                    bf.move_add(bits + index, &[out]);
                }
                bf.zero(discard);
            });

            bf.num_zero(left, width);
            bf.num_zero(right, width);
        });
    }

    /// Write the number at `addr` to standard output in decimal, without
    /// leading zeros.
    pub fn num_print_decimal(&mut self, addr: Addr, width: usize) {
        self.scope(|bf| {
            let value = bf.alloc_zeroed(width);
            let place = bf.alloc_zeroed(width);
            let digit = bf.alloc_zeroed(1);
            let started = bf.alloc_zeroed(1);
            bf.num_copy(addr, value, width);

            let places = decimal_places(width);
            for (index, &power) in places.iter().enumerate() {
                let is_last = index + 1 == places.len();
                bf.zero(digit);

                if is_last {
                    bf.add_copy(value, digit);
                } else {
                    bf.num_set(place, width, power);
                    bf.scope(|bf| {
                        let keep_going = bf.alloc_zeroed(1);
                        bf.num_ge(keep_going, value, place, width);
                        bf.loop_at(keep_going, |bf| {
                            bf.num_sub_assign(value, place, width);
                            bf.add(digit, 1);
                            bf.num_ge(keep_going, value, place, width);
                        });
                    });
                }

                if is_last {
                    bf.set(started, 1);
                } else {
                    bf.if_nonzero(digit, |bf| bf.set(started, 1));
                }

                bf.if_nonzero(started, |bf| {
                    bf.add(digit, b'0' as i32);
                    bf.write(digit);
                    bf.add(digit, -(b'0' as i32));
                });
            }

            bf.zero(digit);
            bf.zero(started);
            bf.num_zero(place, width);
            bf.num_zero(value, width);
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

    fn printed(build: impl FnOnce(&mut Bf)) -> String {
        let mut bf = Bf::new();
        build(&mut bf);
        String::from_utf8(run(&bf.finish())).expect("utf-8 output")
    }

    #[test]
    fn prints_byte_decimals() {
        for value in [0_u8, 7, 10, 99, 100, 255] {
            let text = printed(|bf| {
                let cell = bf.alloc_zeroed(1);
                bf.num_set(cell, 1, u64::from(value));
                bf.num_print_decimal(cell, 1);
            });
            assert_eq!(text, value.to_string());
        }
    }

    #[test]
    fn prints_wide_decimals() {
        for value in [0_u64, 9, 300, 1000, 12_345, 65_535] {
            let text = printed(|bf| {
                let cell = bf.alloc_zeroed(2);
                bf.num_set(cell, 2, value);
                bf.num_print_decimal(cell, 2);
            });
            assert_eq!(text, value.to_string());
        }
    }

    #[test]
    fn adds_and_subtracts_wide_values() {
        for (lhs, rhs) in [
            (1_u64, 2_u64),
            (255, 1),
            (300, 400),
            (65_000, 600),
            (1000, 999),
        ] {
            let sum = printed(|bf| {
                let a = bf.alloc_zeroed(2);
                let b = bf.alloc_zeroed(2);
                bf.num_set(a, 2, lhs);
                bf.num_set(b, 2, rhs);
                bf.num_add_assign(a, b, 2);
                bf.num_print_decimal(a, 2);
            });
            assert_eq!(sum, ((lhs + rhs) % 65_536).to_string(), "{lhs} + {rhs}");

            let difference = printed(|bf| {
                let a = bf.alloc_zeroed(2);
                let b = bf.alloc_zeroed(2);
                bf.num_set(a, 2, lhs);
                bf.num_set(b, 2, rhs);
                bf.num_sub_assign(a, b, 2);
                bf.num_print_decimal(a, 2);
            });
            let expected = (lhs + 65_536 - rhs) % 65_536;
            assert_eq!(difference, expected.to_string(), "{lhs} - {rhs}");
        }
    }

    #[test]
    fn multiplies_and_divides() {
        for (lhs, rhs) in [
            (12_u64, 12_u64),
            (255, 257),
            (1000, 63),
            (65_535, 1),
            (7, 0),
        ] {
            let product = printed(|bf| {
                let a = bf.alloc_zeroed(2);
                let b = bf.alloc_zeroed(2);
                let out = bf.alloc_zeroed(2);
                bf.num_set(a, 2, lhs);
                bf.num_set(b, 2, rhs);
                bf.num_mul(out, a, b, 2);
                bf.num_print_decimal(out, 2);
            });
            assert_eq!(product, (lhs.wrapping_mul(rhs) % 65_536).to_string());

            if let Some(expected) = lhs.checked_div(rhs) {
                let quotient = printed(|bf| {
                    let a = bf.alloc_zeroed(2);
                    let b = bf.alloc_zeroed(2);
                    let q = bf.alloc_zeroed(2);
                    let r = bf.alloc_zeroed(2);
                    bf.num_set(a, 2, lhs);
                    bf.num_set(b, 2, rhs);
                    bf.num_divmod(q, r, a, b, 2);
                    let gap = bf.alloc_zeroed(1);
                    bf.num_print_decimal(q, 2);
                    bf.write_literal(gap, b' ');
                    bf.num_print_decimal(r, 2);
                });
                assert_eq!(quotient, format!("{expected} {}", lhs % rhs));
            }
        }
    }

    #[test]
    fn multiplies_by_constants() {
        for (value, factor) in [
            (0_u64, 10_u64),
            (7, 10),
            (1234, 10),
            (99, 1),
            (300, 200),
            (5, 0),
        ] {
            let text = printed(|bf| {
                let a = bf.alloc_zeroed(2);
                let out = bf.alloc_zeroed(2);
                bf.num_set(a, 2, value);
                bf.num_mul_const(out, a, 2, factor);
                bf.num_print_decimal(out, 2);
            });
            assert_eq!(
                text,
                (value.wrapping_mul(factor) % 65_536).to_string(),
                "{value} * {factor}"
            );
        }
    }

    #[test]
    fn shifts_and_combines_bits() {
        let text = printed(|bf| {
            let a = bf.alloc_zeroed(2);
            let b = bf.alloc_zeroed(2);
            let out = bf.alloc_zeroed(2);
            bf.num_set(a, 2, 0b1100_1010);
            bf.num_set(b, 2, 0b1010_0110);
            bf.num_bitwise(out, a, b, 2, BitKind::Xor);
            bf.num_print_decimal(out, 2);
        });
        assert_eq!(text, (0b1100_1010_u64 ^ 0b1010_0110).to_string());

        let shifted = printed(|bf| {
            let a = bf.alloc_zeroed(2);
            bf.num_set(a, 2, 1234);
            bf.num_shl_const(a, 2, 3);
            bf.num_print_decimal(a, 2);
        });
        assert_eq!(shifted, (1234_u64 << 3).to_string());
    }

    #[test]
    fn orders_wide_values() {
        for (lhs, rhs) in [
            (0_u64, 0_u64),
            (1, 256),
            (256, 1),
            (65_535, 65_534),
            (300, 300),
        ] {
            let text = printed(|bf| {
                let a = bf.alloc_zeroed(2);
                let b = bf.alloc_zeroed(2);
                let out = bf.alloc_zeroed(1);
                bf.num_set(a, 2, lhs);
                bf.num_set(b, 2, rhs);
                bf.num_lt(out, a, b, 2);
                bf.add(out, b'0' as i32);
                bf.write(out);
                bf.num_eq(out, a, b, 2);
                bf.add(out, b'0' as i32);
                bf.write(out);
            });
            let expected = format!("{}{}", u8::from(lhs < rhs), u8::from(lhs == rhs));
            assert_eq!(text, expected, "comparing {lhs} and {rhs}");
        }
    }
}
