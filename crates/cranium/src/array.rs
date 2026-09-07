//! Tape layout and runtime indexing for Cranium arrays.
//!
//! Brainfuck cannot compute an address, so indexing by a value only known at
//! run time has to be done by *walking*: the index is carried rightwards one
//! element at a time until it runs out, the element is read or written there,
//! and a trail of breadcrumbs leads the pointer back. That is why an array
//! element costs more than one cell.
//!
//! Each element gets a slot of `stride` cells:
//!
//! | lane | width | purpose |
//! | --- | --- | --- |
//! | `go` | 1 | loop flag; nonzero while the walk should continue |
//! | `crumb` | 1 | breadcrumb marking a slot the walk passed through |
//! | `count` | index width | how many slots are left to travel |
//! | `scratch` | 0 or 2 | borrow bookkeeping for wide index decrements |
//! | `carry` | element width | the value in transit, outbound or inbound |
//! | `data` | element width | the element itself |
//!
//! For arrays of at most 256 elements the counter is a single cell, and "the
//! counter is nonzero" is exactly "keep walking" - so the two lanes share one
//! cell and each step of the walk gets shorter. Wider arrays need a separate
//! flag because a multi-cell counter cannot be tested by one `[`.
//!
//! A guard slot follows the last element, and `stride - 1` zero cells precede
//! the first one because the return walk steps one slot past the front before
//! noticing it has arrived.

use crate::bf::{Addr, Bf};

/// How one array is laid out on the tape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrayLayout {
    /// Number of elements the array holds.
    pub length: usize,
    /// Cells per element value.
    pub element_width: usize,
    /// Cells used by the travelling index counter.
    pub index_width: usize,
    /// Cells reserved for wide-counter borrow handling.
    pub scratch_width: usize,
    /// Cells per slot.
    pub stride: Addr,
}

/// Longest array that a one-cell index counter can address.
pub const MAX_BYTE_INDEXED_LENGTH: usize = 256;

impl ArrayLayout {
    /// Choose a layout for `length` elements of `element_width` cells each.
    pub fn new(length: usize, element_width: usize) -> Self {
        let index_width = if length <= MAX_BYTE_INDEXED_LENGTH {
            1
        } else {
            2
        };
        let scratch_width = if index_width > 1 { 2 } else { 0 };
        // A one-cell counter doubles as the walk flag, so it costs no lane.
        let counter_lane = if index_width > 1 { index_width } else { 0 };
        let stride = (2 + counter_lane + scratch_width + 2 * element_width) as Addr;
        Self {
            length,
            element_width,
            index_width,
            scratch_width,
            stride,
        }
    }

    /// Total cells to reserve, including leading pad and the guard slot.
    pub fn region_cells(&self) -> usize {
        (self.stride - 1) as usize + (self.length + 1) * self.stride as usize
    }

    /// Distance from the start of the reserved region to slot zero.
    pub fn base_offset(&self) -> Addr {
        self.stride - 1
    }

    /// Offset of the walk flag inside a slot.
    pub fn go(&self) -> Addr {
        0
    }

    /// Offset of the breadcrumb inside a slot.
    pub fn crumb(&self) -> Addr {
        1
    }

    /// True when the walk flag and the counter occupy the same cell.
    pub fn flag_is_counter(&self) -> bool {
        self.index_width == 1
    }

    /// Offset of the travelling counter inside a slot.
    pub fn count(&self) -> Addr {
        if self.flag_is_counter() { self.go() } else { 2 }
    }

    /// Offset of the borrow scratch lane inside a slot.
    pub fn scratch(&self) -> Addr {
        2 + self.index_width as Addr
    }

    /// Offset of the value-in-transit lane inside a slot.
    pub fn carry(&self) -> Addr {
        if self.flag_is_counter() {
            2
        } else {
            self.scratch() + self.scratch_width as Addr
        }
    }

    /// Offset of the stored element inside a slot.
    pub fn data(&self) -> Addr {
        self.carry() + self.element_width as Addr
    }

    /// Address of element `index`, for indices known at compile time.
    pub fn element(&self, base: Addr, index: usize) -> Addr {
        base + index as Addr * self.stride + self.data()
    }

    /// Address of slot zero's counter lane.
    pub fn counter(&self, base: Addr) -> Addr {
        base + self.count()
    }

    /// Address of slot zero's value-in-transit lane.
    pub fn transit(&self, base: Addr) -> Addr {
        base + self.carry()
    }
}

/// Builds Brainfuck whose moves are relative to a frame that shifts at run
/// time.
struct Relative {
    out: String,
    at: Addr,
}

impl Relative {
    fn new() -> Self {
        Self {
            out: String::new(),
            at: 0,
        }
    }

    fn go(&mut self, to: Addr) {
        let delta = to - self.at;
        let (ch, count) = if delta >= 0 {
            ('>', delta)
        } else {
            ('<', -delta)
        };
        for _ in 0..count {
            self.out.push(ch);
        }
        self.at = to;
    }

    fn put(&mut self, code: &str) {
        self.out.push_str(code);
    }

    fn at(&mut self, cell: Addr, code: &str) {
        self.go(cell);
        self.put(code);
    }
}

/// Whether an indexed access reads the element or overwrites it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Fetch the element into slot zero's transit lane.
    Read,
    /// Store slot zero's transit lane into the element.
    Write,
}

impl Bf {
    /// Emit a dynamically indexed array access.
    ///
    /// On entry the counter lane of slot zero holds the index, the walk flag
    /// holds whether the index is nonzero, and for a write the transit lane
    /// holds the value. On return every bookkeeping lane is zero again, the
    /// data pointer is back at slot zero's transit lane, and for a read that
    /// lane holds the element.
    pub fn array_access(&mut self, base: Addr, layout: &ArrayLayout, access: Access) {
        let stride = layout.stride;
        let mut code = String::new();

        code.push('[');
        code.push_str(&walk_out_body(layout, access));
        code.push(']');

        code.push_str(&access_body(layout, access));

        let mut walk_back = Relative::new();
        walk_back.go(layout.crumb() - stride);
        walk_back.put("[");
        walk_back.put(&walk_back_body(layout, access));
        walk_back.put("]");
        code.push_str(&walk_back.out);

        // The return walk stops one slot in front of the array, so step back
        // onto the transit lane of slot zero.
        let mut landing = Relative::new();
        landing.go(layout.carry() - layout.crumb() + stride);
        code.push_str(&landing.out);

        self.goto(base + layout.go());
        self.emit_raw(&code);
        self.set_pos(base + layout.carry());
    }
}

/// One step of the outbound walk, entered with the pointer on a slot's flag.
fn walk_out_body(layout: &ArrayLayout, access: Access) -> String {
    let stride = layout.stride;
    let mut r = Relative::new();

    if !layout.flag_is_counter() {
        r.at(layout.go(), "[-]");
    }
    r.at(layout.crumb(), "+");

    if layout.index_width == 1 {
        r.at(layout.count(), "-");
    } else {
        let low = layout.count();
        let high = layout.count() + 1;
        let spare = layout.scratch();
        let saved = layout.scratch() + 1;

        r.at(low, "[-");
        r.at(spare, "+");
        r.at(saved, "+");
        r.at(low, "]");
        r.at(spare, "[-");
        r.at(low, "+");
        r.at(spare, "]");
        // `spare` now asks "was the low cell zero?", which is what decides the
        // borrow into the high cell.
        r.at(spare, "[-]+");
        r.at(saved, "[");
        r.at(spare, "[-]");
        r.at(saved, "[-]");
        r.put("]");
        r.at(spare, "[");
        r.put("[-]");
        r.at(high, "-");
        r.at(spare, "]");
        r.at(low, "-");
    }

    // Hand the remaining count to the next slot. When the counter is also the
    // walk flag the move is all that is needed; otherwise the flag is raised in
    // the same pass so no extra scratch cell is required.
    for cell in 0..layout.index_width as Addr {
        let here = layout.count() + cell;
        r.at(here, "[-");
        r.at(here + stride, "+");
        if !layout.flag_is_counter() {
            r.at(stride + layout.go(), "[-]+");
        }
        r.at(here, "]");
    }

    if access == Access::Write {
        for cell in 0..layout.element_width as Addr {
            let here = layout.carry() + cell;
            r.at(here, "[-");
            r.at(here + stride, "+");
            r.at(here, "]");
        }
    }

    r.go(stride + layout.go());
    r.out
}

/// The read or write performed once the walk has arrived.
fn access_body(layout: &ArrayLayout, access: Access) -> String {
    let mut r = Relative::new();

    for cell in 0..layout.element_width as Addr {
        let data = layout.data() + cell;
        let carry = layout.carry() + cell;
        match access {
            Access::Read => {
                // The counter lane is spent by now, so it is free scratch.
                let spare = layout.count();
                r.at(data, "[-");
                r.at(carry, "+");
                r.at(spare, "+");
                r.at(data, "]");
                r.at(spare, "[-");
                r.at(data, "+");
                r.at(spare, "]");
            }
            Access::Write => {
                r.at(data, "[-]");
                r.at(carry, "[-");
                r.at(data, "+");
                r.at(carry, "]");
            }
        }
    }

    r.go(layout.go());
    r.out
}

/// One step of the return walk, entered with the pointer on a breadcrumb.
fn walk_back_body(layout: &ArrayLayout, access: Access) -> String {
    let stride = layout.stride;
    let mut r = Relative::new();
    r.put("-");

    if access == Access::Read {
        for cell in 0..layout.element_width as Addr {
            let ahead = layout.carry() + cell - layout.crumb() + stride;
            let behind = layout.carry() + cell - layout.crumb();
            r.at(ahead, "[-");
            r.at(behind, "+");
            r.at(ahead, "]");
        }
    }

    r.go(-stride);
    r.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lobe::{CellSize, create_runtime};

    fn run(src: &str) -> Vec<u8> {
        let mut runtime = create_runtime(src, CellSize::Bits8).expect("valid brainfuck");
        let mut input = std::io::Cursor::new(Vec::new());
        let mut output = Vec::new();
        runtime
            .run_with_io(&mut input, &mut output)
            .expect("program runs");
        output
    }

    /// Store `values` into a fresh array, then read one element back by an
    /// index the generated program only learns at run time.
    fn read_back(values: &[u8], index: u8, length: usize) -> u8 {
        let layout = ArrayLayout::new(length, 1);
        let mut bf = Bf::new();
        let index_cell = bf.alloc_zeroed(1);
        let result = bf.alloc_zeroed(1);
        let region = bf.alloc_zeroed(layout.region_cells());
        let base = region + layout.base_offset();

        for (slot, &value) in values.iter().enumerate() {
            bf.set(layout.element(base, slot), value);
        }

        bf.set(index_cell, index);
        bf.copy(index_cell, layout.counter(base));
        bf.array_access(base, &layout, Access::Read);
        bf.move_add(layout.transit(base), &[result]);
        bf.write(result);

        run(&bf.finish())[0]
    }

    #[test]
    fn reads_elements_by_runtime_index() {
        let values: Vec<u8> = (0..16).map(|index| index * 3 + 1).collect();
        for index in [0_u8, 1, 7, 15] {
            assert_eq!(
                read_back(&values, index, values.len()),
                values[index as usize],
                "index {index}"
            );
        }
    }

    #[test]
    fn writes_elements_by_runtime_index() {
        let layout = ArrayLayout::new(8, 1);
        let mut bf = Bf::new();
        let value = bf.alloc_zeroed(1);
        let region = bf.alloc_zeroed(layout.region_cells());
        let base = region + layout.base_offset();

        for slot in 0..8 {
            bf.set(layout.element(base, slot), b'.');
        }
        for (slot, byte) in [(5_u8, b'X'), (0, b'A'), (7, b'Z')] {
            bf.set(value, byte);
            bf.set(layout.counter(base), slot);
            bf.copy(value, layout.transit(base));
            bf.array_access(base, &layout, Access::Write);
        }
        for slot in 0..8 {
            bf.write(layout.element(base, slot));
        }

        assert_eq!(run(&bf.finish()), b"A....X.Z".to_vec());
    }

    #[test]
    fn walks_wide_arrays_with_a_two_cell_counter() {
        let layout = ArrayLayout::new(400, 1);
        assert_eq!(layout.index_width, 2);

        let mut bf = Bf::new();
        let index = bf.alloc_zeroed(2);
        let region = bf.alloc_zeroed(layout.region_cells());
        let base = region + layout.base_offset();

        bf.set(layout.element(base, 300), b'!');
        bf.set(layout.element(base, 0), b'?');

        for target in [300_u64, 0, 257] {
            bf.num_set(index, 2, target);
            bf.num_copy(index, layout.counter(base), 2);
            bf.num_is_nonzero(base + layout.go(), layout.counter(base), 2);
            bf.array_access(base, &layout, Access::Read);
            bf.write(layout.transit(base));
            bf.zero(layout.transit(base));
        }

        assert_eq!(run(&bf.finish()), vec![b'!', b'?', 0]);
    }

    #[test]
    fn handles_two_cell_elements() {
        let layout = ArrayLayout::new(6, 2);
        let mut bf = Bf::new();
        let region = bf.alloc_zeroed(layout.region_cells());
        let base = region + layout.base_offset();

        bf.num_set(layout.element(base, 4), 2, 1000);
        bf.set(layout.counter(base), 4);
        bf.array_access(base, &layout, Access::Read);
        bf.num_print_decimal(layout.transit(base), 2);
        bf.num_zero(layout.transit(base), 2);

        assert_eq!(run(&bf.finish()), b"1000".to_vec());
    }
}
