//! Lowering from the Cranium AST to Brainfuck.
//!
//! There is no call stack on a Brainfuck tape, so every call is inlined and
//! recursion is a compile-time error. Everything else is ordinary: each
//! variable owns a fixed run of cells, expressions evaluate into temporaries
//! that are released at the end of the statement, and structured control flow
//! becomes nested Brainfuck loops.
//!
//! `break`, `continue`, and `return` need a little more: Brainfuck has no jump.
//! A single *control cell* records why execution stopped (`1` continue, `2`
//! break, `3` return), loops fold it into their condition, and the statements
//! after an interruptible statement are wrapped in a "still running?" guard.
//! Blocks that contain none of those keywords pay nothing for the machinery.

use crate::array::{Access, ArrayLayout};
use crate::ast::{BinOp, Expr, Function, Item, Param, Program, Stmt, Type, UnOp};
use crate::bf::{Addr, Bf};
use crate::lexer::Span;
use crate::num::BitKind;
use std::collections::HashMap;
use std::fmt;

/// A compiled program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The emitted Brainfuck.
    pub code: String,
    /// Highest tape cell the program can reach.
    pub cells_used: usize,
}

/// A lowering failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    /// Human readable description.
    pub message: String,
    /// Where the problem was found.
    pub span: Span,
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.span, self.message)
    }
}

impl std::error::Error for CompileError {}

type CResult<T> = Result<T, CompileError>;

fn error<T>(span: Span, message: impl Into<String>) -> CResult<T> {
    Err(CompileError {
        message: message.into(),
        span,
    })
}

/// Reasons execution left a block early, as stored in the control cell.
const CONTROL_CONTINUE: u8 = 1;
const CONTROL_BREAK: u8 = 2;
const CONTROL_RETURN: u8 = 3;

/// Builtins the compiler implements directly.
const BUILTINS: &[&str] = &["print", "println", "putc", "getc", "puts", "len"];

#[derive(Debug, Clone)]
struct Binding {
    addr: Addr,
    ty: Type,
}

#[derive(Debug, Default)]
struct Scope {
    names: HashMap<String, Binding>,
    /// True for the scope a function body starts with, which hides the
    /// caller's locals.
    barrier: bool,
}

/// An evaluated expression. The cells are readable but must not be written to;
/// operations that produce values always allocate their own destination.
#[derive(Debug, Clone)]
struct Value {
    addr: Addr,
    ty: Type,
}

/// Somewhere a value can be stored.
enum Place {
    /// A variable, or an array element whose index is known at compile time.
    Direct { addr: Addr, ty: Type },
    /// An array element reached by walking the tape at run time.
    Indexed {
        base: Addr,
        layout: ArrayLayout,
        element: Type,
        index: Value,
    },
}

impl Place {
    fn ty(&self) -> &Type {
        match self {
            Place::Direct { ty, .. } => ty,
            Place::Indexed { element, .. } => element,
        }
    }
}

struct Compiler<'a> {
    bf: Bf,
    functions: HashMap<String, &'a Function>,
    constants: HashMap<String, (Type, u64)>,
    scopes: Vec<Scope>,
    inlining: Vec<String>,
    returns: Vec<(Addr, Type)>,
    loop_depth: usize,
    control: Addr,
}

/// Lower a parsed program to Brainfuck.
///
/// # Errors
///
/// Returns a [`CompileError`] describing the first problem found.
pub fn compile(program: &Program) -> CResult<Output> {
    let mut compiler = Compiler::new();
    compiler.run(program)?;
    let cells_used = compiler.bf.cells_used();
    Ok(Output {
        code: compiler.bf.finish(),
        cells_used,
    })
}

impl<'a> Compiler<'a> {
    fn new() -> Self {
        Self {
            bf: Bf::new(),
            functions: HashMap::new(),
            constants: HashMap::new(),
            scopes: vec![Scope::default()],
            inlining: Vec::new(),
            returns: Vec::new(),
            loop_depth: 0,
            control: 0,
        }
    }

    fn run(&mut self, program: &'a Program) -> CResult<()> {
        for item in &program.items {
            if let Item::Function(function) = item {
                if BUILTINS.contains(&function.name.as_str()) {
                    return error(
                        function.span,
                        format!("`{}` is a builtin and cannot be redefined", function.name),
                    );
                }
                if self
                    .functions
                    .insert(function.name.clone(), function)
                    .is_some()
                {
                    return error(
                        function.span,
                        format!("`{}` is defined more than once", function.name),
                    );
                }
            }
        }

        let Some(main) = self.functions.get("main").copied() else {
            return error(Span { line: 1, column: 1 }, "no `main` function");
        };
        if !main.params.is_empty() {
            return error(main.span, "`main` cannot take parameters");
        }

        self.control = self.bf.alloc_zeroed(1);

        for item in &program.items {
            match item {
                Item::Const {
                    name,
                    ty,
                    value,
                    span,
                } => {
                    let declared = ty.clone().unwrap_or(self.type_of(value)?);
                    if !declared.is_scalar() {
                        return error(*span, "constants must be `byte`, `int`, or `bool`");
                    }
                    let evaluated = self.const_eval(value)?;
                    if evaluated > declared.max_value() {
                        return error(*span, format!("{evaluated} does not fit in `{declared}`"));
                    }
                    if self
                        .constants
                        .insert(name.clone(), (declared, evaluated))
                        .is_some()
                    {
                        return error(*span, format!("`{name}` is defined more than once"));
                    }
                }
                Item::Global {
                    name,
                    ty,
                    init,
                    span,
                } => {
                    self.declare(name, ty.as_ref(), init.as_ref(), *span)?;
                }
                Item::Function(_) => {}
            }
        }

        self.inline_call(main, &[], main.span)?;
        Ok(())
    }

    // ---- scopes -----------------------------------------------------------

    fn lookup(&self, name: &str) -> Option<&Binding> {
        for (index, scope) in self.scopes.iter().enumerate().rev() {
            if let Some(binding) = scope.names.get(name) {
                return Some(binding);
            }
            if scope.barrier && index > 0 {
                return self.scopes[0].names.get(name);
            }
        }
        None
    }

    fn bind(&mut self, name: &str, binding: Binding) {
        self.scopes
            .last_mut()
            .expect("a scope is always open")
            .names
            .insert(name.to_string(), binding);
    }

    fn push_scope(&mut self, barrier: bool) {
        self.scopes.push(Scope {
            names: HashMap::new(),
            barrier,
        });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    // ---- types ------------------------------------------------------------

    fn type_of(&self, expr: &Expr) -> CResult<Type> {
        Ok(match expr {
            Expr::Int { value, .. } => {
                if *value <= 255 {
                    Type::Byte
                } else {
                    Type::Int
                }
            }
            Expr::Bool { .. } => Type::Bool,
            Expr::Str { value, span } => {
                if value.len() + 1 > u16::MAX as usize {
                    return error(*span, "string literal is too long");
                }
                Type::Array {
                    element: Box::new(Type::Byte),
                    length: value.len() + 1,
                }
            }
            Expr::ArrayLit { elements, span } => {
                if elements.is_empty() {
                    return error(*span, "array literals need at least one element");
                }
                let mut element = Type::Byte;
                for item in elements {
                    let item_ty = promote(&self.type_of(item)?);
                    if !item_ty.is_scalar() {
                        return error(item.span(), "array elements must be scalars");
                    }
                    element = wider(&element, &item_ty);
                }
                Type::Array {
                    element: Box::new(element),
                    length: elements.len(),
                }
            }
            Expr::Name { name, span } => {
                if let Some(binding) = self.lookup(name) {
                    binding.ty.clone()
                } else if let Some((ty, _)) = self.constants.get(name) {
                    ty.clone()
                } else {
                    return error(*span, format!("`{name}` is not defined"));
                }
            }
            Expr::Index { array, span, .. } => match self.type_of(array)? {
                Type::Array { element, .. } => *element,
                other => return error(*span, format!("`{other}` cannot be indexed")),
            },
            Expr::Call { name, span, .. } => match name.as_str() {
                "print" | "println" | "putc" | "puts" => Type::Unit,
                "getc" => Type::Byte,
                "len" => Type::Int,
                _ => match self.functions.get(name) {
                    Some(function) => function.ret.clone(),
                    None => return error(*span, format!("`{name}` is not defined")),
                },
            },
            Expr::Binary { op, lhs, rhs, .. } => {
                if op.is_comparison() || matches!(op, BinOp::And | BinOp::Or) {
                    Type::Bool
                } else if matches!(op, BinOp::Shl | BinOp::Shr) {
                    promote(&self.type_of(lhs)?)
                } else {
                    wider(&promote(&self.type_of(lhs)?), &promote(&self.type_of(rhs)?))
                }
            }
            Expr::Unary { op, operand, .. } => match op {
                UnOp::Not => Type::Bool,
                UnOp::Neg => promote(&self.type_of(operand)?),
            },
            Expr::Cast { ty, .. } => ty.clone(),
        })
    }

    fn const_eval(&self, expr: &Expr) -> CResult<u64> {
        Ok(match expr {
            Expr::Int { value, .. } => *value,
            Expr::Bool { value, .. } => u64::from(*value),
            Expr::Name { name, span } => match self.constants.get(name) {
                Some((_, value)) => *value,
                None => return error(*span, format!("`{name}` is not a compile-time constant")),
            },
            Expr::Unary { op, operand, .. } => {
                let value = self.const_eval(operand)?;
                match op {
                    UnOp::Not => u64::from(value == 0),
                    UnOp::Neg => value.wrapping_neg() & 0xffff,
                }
            }
            Expr::Cast { value, ty, .. } => {
                let value = self.const_eval(value)?;
                match ty {
                    Type::Bool => u64::from(value != 0),
                    other => value & other.max_value(),
                }
            }
            Expr::Binary { op, lhs, rhs, span } => {
                let left = self.const_eval(lhs)?;
                let right = self.const_eval(rhs)?;
                match op {
                    BinOp::Add => left.wrapping_add(right),
                    BinOp::Sub => left.wrapping_sub(right),
                    BinOp::Mul => left.wrapping_mul(right),
                    BinOp::Div | BinOp::Rem if right == 0 => {
                        return error(*span, "constant division by zero")
                    }
                    BinOp::Div => left / right,
                    BinOp::Rem => left % right,
                    BinOp::Eq => u64::from(left == right),
                    BinOp::Ne => u64::from(left != right),
                    BinOp::Lt => u64::from(left < right),
                    BinOp::Le => u64::from(left <= right),
                    BinOp::Gt => u64::from(left > right),
                    BinOp::Ge => u64::from(left >= right),
                    BinOp::And => u64::from(left != 0 && right != 0),
                    BinOp::Or => u64::from(left != 0 || right != 0),
                    BinOp::BitAnd => left & right,
                    BinOp::BitOr => left | right,
                    BinOp::BitXor => left ^ right,
                    BinOp::Shl => left.wrapping_shl(right as u32),
                    BinOp::Shr => left.wrapping_shr(right as u32),
                }
            }
            other => {
                return error(other.span(), "this is not a compile-time constant");
            }
        })
    }

    // ---- declarations -----------------------------------------------------

    fn declare(
        &mut self,
        name: &str,
        declared: Option<&Type>,
        init: Option<&'a Expr>,
        span: Span,
    ) -> CResult<()> {
        let ty = match (declared, init) {
            (Some(ty), _) => ty.clone(),
            (None, Some(expr)) => self.type_of(expr)?,
            (None, None) => return error(span, format!("`{name}` needs a type or an initializer")),
        };

        match &ty {
            Type::Array { element, length } => {
                let element = (**element).clone();
                self.declare_array(name, &element, *length, init, span)
            }
            Type::Unit => error(span, "variables cannot have type `()`"),
            scalar => {
                let addr = self.bf.alloc(scalar.width());
                self.bf.num_zero(addr, scalar.width());
                if let Some(expr) = init {
                    let mark = self.bf.watermark();
                    let value = self.eval(expr)?;
                    self.store_scalar(addr, scalar, &value, expr.span())?;
                    self.bf.release_to(mark);
                }
                self.bind(name, Binding { addr, ty });
                Ok(())
            }
        }
    }

    fn declare_array(
        &mut self,
        name: &str,
        element: &Type,
        length: usize,
        init: Option<&'a Expr>,
        span: Span,
    ) -> CResult<()> {
        if !element.is_scalar() {
            return error(span, "arrays must hold `byte`, `int`, or `bool`");
        }
        let layout = ArrayLayout::new(length, element.width());
        let region = self.bf.alloc(layout.region_cells());
        for offset in 0..layout.region_cells() as Addr {
            self.bf.zero(region + offset);
        }
        let base = region + layout.base_offset();

        match init {
            None => {}
            Some(Expr::Str { value, span }) => {
                if !matches!(element, Type::Byte) {
                    return error(*span, "string initializers only work for `byte` arrays");
                }
                if value.len() + 1 > length {
                    return error(
                        *span,
                        format!(
                            "string needs {} elements including the terminator, but `{name}` holds {length}",
                            value.len() + 1
                        ),
                    );
                }
                for (index, &byte) in value.iter().enumerate() {
                    self.bf.set(layout.element(base, index), byte);
                }
            }
            Some(Expr::ArrayLit { elements, span }) => {
                if elements.len() > length {
                    return error(
                        *span,
                        format!(
                            "literal has {} elements but `{name}` holds {length}",
                            elements.len()
                        ),
                    );
                }
                for (index, item) in elements.iter().enumerate() {
                    let value = self.const_eval(item)?;
                    if value > element.max_value() {
                        return error(item.span(), format!("{value} does not fit in `{element}`"));
                    }
                    self.bf
                        .num_set(layout.element(base, index), element.width(), value);
                }
            }
            Some(other) => {
                return error(
                    other.span(),
                    "arrays can only be initialized with a string or an array literal",
                )
            }
        }

        self.bind(
            name,
            Binding {
                addr: base,
                ty: Type::Array {
                    element: Box::new(element.clone()),
                    length,
                },
            },
        );
        Ok(())
    }

    // ---- statements -------------------------------------------------------

    fn block(&mut self, body: &'a [Stmt]) -> CResult<()> {
        let mark = self.bf.watermark();
        self.push_scope(false);
        let result = self.statements(body);
        self.pop_scope();
        self.bf.release_to(mark);
        result
    }

    fn statements(&mut self, body: &'a [Stmt]) -> CResult<()> {
        for (index, stmt) in body.iter().enumerate() {
            self.statement(stmt)?;
            if index + 1 < body.len() && interrupts(stmt) {
                return self.guard_running(&body[index + 1..]);
            }
        }
        Ok(())
    }

    /// Emit `rest` only when the control cell says nothing has interrupted us.
    fn guard_running(&mut self, rest: &'a [Stmt]) -> CResult<()> {
        let flag = self.bf.alloc_zeroed(1);
        self.bf.is_zero(flag, self.control);
        self.bf.open_loop(flag);
        let mark = self.bf.watermark();
        let result = self.statements(rest);
        self.bf.release_to(mark);
        self.bf.zero(flag);
        self.bf.close_loop(flag);
        result
    }

    fn statement(&mut self, stmt: &'a Stmt) -> CResult<()> {
        match stmt {
            Stmt::Let {
                name,
                ty,
                init,
                span,
            } => self.declare(name, ty.as_ref(), init.as_ref(), *span),

            Stmt::Block { body, .. } => self.block(body),

            Stmt::Expr { expr, .. } => {
                let mark = self.bf.watermark();
                self.eval(expr)?;
                self.bf.release_to(mark);
                Ok(())
            }

            Stmt::Assign {
                target,
                op,
                value,
                span,
            } => self.assign(target, *op, value, *span),

            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => self.emit_if(cond, then_block, else_block.as_deref()),

            Stmt::While { cond, body, .. } => self.emit_loop(Some(cond), body),

            Stmt::Loop { body, .. } => self.emit_loop(None, body),

            Stmt::For {
                name,
                start,
                end,
                body,
                span,
            } => self.emit_for(name, start, end, body, *span),

            Stmt::Break { span } => {
                if self.loop_depth == 0 {
                    return error(*span, "`break` outside a loop");
                }
                self.bf.set(self.control, CONTROL_BREAK);
                Ok(())
            }

            Stmt::Continue { span } => {
                if self.loop_depth == 0 {
                    return error(*span, "`continue` outside a loop");
                }
                self.bf.set(self.control, CONTROL_CONTINUE);
                Ok(())
            }

            Stmt::Return { value, span } => {
                let Some((slot, ret_ty)) = self.returns.last().cloned() else {
                    return error(*span, "`return` outside a function");
                };
                match (value, &ret_ty) {
                    (None, Type::Unit) => {}
                    (None, other) => {
                        return error(*span, format!("this function must return `{other}`"))
                    }
                    (Some(expr), Type::Unit) => {
                        return error(expr.span(), "this function does not return a value")
                    }
                    (Some(expr), other) => {
                        let mark = self.bf.watermark();
                        let evaluated = self.eval(expr)?;
                        self.store_scalar(slot, other, &evaluated, expr.span())?;
                        self.bf.release_to(mark);
                    }
                }
                self.bf.set(self.control, CONTROL_RETURN);
                Ok(())
            }
        }
    }

    fn assign(
        &mut self,
        target: &'a Expr,
        op: Option<BinOp>,
        value: &'a Expr,
        span: Span,
    ) -> CResult<()> {
        let mark = self.bf.watermark();
        let place = self.place(target)?;
        let ty = place.ty().clone();
        if !ty.is_scalar() {
            self.bf.release_to(mark);
            return error(span, "only scalar values can be assigned");
        }

        let stored = self.bf.alloc_zeroed(ty.width());
        let inner = self.bf.watermark();

        match op {
            None => {
                let evaluated = self.eval(value)?;
                self.store_scalar(stored, &ty, &evaluated, value.span())?;
            }
            Some(op) => {
                let current = self.load(&place)?;
                let combined = self.binary(op, &current, value, span)?;
                self.store_scalar(stored, &ty, &combined, span)?;
            }
        }

        self.bf.release_to(inner);
        self.store(
            &place,
            &Value {
                addr: stored,
                ty: ty.clone(),
            },
            span,
        )?;
        self.bf.release_to(mark);
        Ok(())
    }

    fn emit_if(
        &mut self,
        cond: &'a Expr,
        then_block: &'a [Stmt],
        else_block: Option<&'a [Stmt]>,
    ) -> CResult<()> {
        let taken = self.bf.alloc_zeroed(1);
        let skipped = self.bf.alloc_zeroed(1);
        let mark = self.bf.watermark();
        let evaluated = self.eval(cond)?;
        self.truthy(&evaluated, taken, cond.span())?;
        self.bf.release_to(mark);
        self.bf.set(skipped, 1);

        self.bf.open_loop(taken);
        self.block(then_block)?;
        self.bf.zero(skipped);
        self.bf.zero(taken);
        self.bf.close_loop(taken);

        self.bf.open_loop(skipped);
        if let Some(body) = else_block {
            self.block(body)?;
        }
        self.bf.zero(skipped);
        self.bf.close_loop(skipped);
        Ok(())
    }

    /// Evaluate a loop condition, folding in "and nothing has interrupted us".
    fn loop_condition(&mut self, cond: Option<&'a Expr>, out: Addr) -> CResult<()> {
        let mark = self.bf.watermark();
        match cond {
            Some(expr) => {
                let evaluated = self.eval(expr)?;
                self.truthy(&evaluated, out, expr.span())?;
            }
            None => self.bf.set(out, 1),
        }
        let control = self.control;
        self.bf.if_nonzero(control, |bf| bf.zero(out));
        self.bf.release_to(mark);
        Ok(())
    }

    fn emit_loop(&mut self, cond: Option<&'a Expr>, body: &'a [Stmt]) -> CResult<()> {
        let running = self.bf.alloc_zeroed(1);
        self.loop_condition(cond, running)?;

        self.bf.open_loop(running);
        self.loop_depth += 1;
        let result = self.block(body);
        self.loop_depth -= 1;
        result?;
        self.clear_control(CONTROL_CONTINUE);
        self.loop_condition(cond, running)?;
        self.bf.close_loop(running);

        self.clear_control(CONTROL_BREAK);
        Ok(())
    }

    fn emit_for(
        &mut self,
        name: &str,
        start: &'a Expr,
        end: &'a Expr,
        body: &'a [Stmt],
        span: Span,
    ) -> CResult<()> {
        let ty = wider(
            &promote(&self.type_of(start)?),
            &promote(&self.type_of(end)?),
        );
        if !ty.is_scalar() {
            return error(span, "`for` bounds must be scalars");
        }
        let width = ty.width();

        let mark = self.bf.watermark();
        let counter = self.bf.alloc_zeroed(width);
        let limit = self.bf.alloc_zeroed(width);
        let running = self.bf.alloc_zeroed(1);

        let inner = self.bf.watermark();
        let start_value = self.eval(start)?;
        self.store_scalar(counter, &ty, &start_value, start.span())?;
        let end_value = self.eval(end)?;
        self.store_scalar(limit, &ty, &end_value, end.span())?;
        self.bf.release_to(inner);

        self.push_scope(false);
        self.bind(
            name,
            Binding {
                addr: counter,
                ty: ty.clone(),
            },
        );

        self.bf.num_lt(running, counter, limit, width);
        let control = self.control;
        self.bf.if_nonzero(control, |bf| bf.zero(running));

        self.bf.open_loop(running);
        self.loop_depth += 1;
        let result = self.block(body);
        self.loop_depth -= 1;
        result?;
        self.clear_control(CONTROL_CONTINUE);
        self.increment(counter, width);
        self.bf.num_lt(running, counter, limit, width);
        self.bf.if_nonzero(control, |bf| bf.zero(running));
        self.bf.close_loop(running);

        self.clear_control(CONTROL_BREAK);
        self.pop_scope();
        self.bf.release_to(mark);
        Ok(())
    }

    fn increment(&mut self, addr: Addr, width: usize) {
        if width == 1 {
            self.bf.add(addr, 1);
            return;
        }
        let mark = self.bf.watermark();
        let one = self.bf.alloc_zeroed(width);
        self.bf.num_set(one, width, 1);
        self.bf.num_add_assign(addr, one, width);
        self.bf.num_zero(one, width);
        self.bf.release_to(mark);
    }

    /// Reset the control cell if it currently holds `reason`.
    fn clear_control(&mut self, reason: u8) {
        let mark = self.bf.watermark();
        let matches = self.bf.alloc_zeroed(1);
        let expected = self.bf.alloc_zeroed(1);
        self.bf.set(expected, reason);
        let control = self.control;
        self.bf.byte_eq(matches, control, expected);
        self.bf.if_nonzero_consume(matches, |bf| bf.zero(control));
        self.bf.zero(expected);
        self.bf.release_to(mark);
    }

    // ---- places -----------------------------------------------------------

    fn place(&mut self, expr: &'a Expr) -> CResult<Place> {
        match expr {
            Expr::Name { name, span } => {
                if self.constants.contains_key(name) {
                    return error(
                        *span,
                        format!("`{name}` is a constant and cannot be assigned"),
                    );
                }
                match self.lookup(name) {
                    Some(binding) => Ok(Place::Direct {
                        addr: binding.addr,
                        ty: binding.ty.clone(),
                    }),
                    None => error(*span, format!("`{name}` is not defined")),
                }
            }
            Expr::Index { array, index, span } => {
                let Expr::Name { name, .. } = &**array else {
                    return error(*span, "only named arrays can be indexed");
                };
                let Some(binding) = self.lookup(name).cloned() else {
                    return error(*span, format!("`{name}` is not defined"));
                };
                let Type::Array { element, length } = binding.ty.clone() else {
                    return error(*span, format!("`{name}` is not an array"));
                };
                let layout = ArrayLayout::new(length, element.width());

                if let Ok(constant) = self.const_eval(index) {
                    if constant as usize >= length {
                        return error(
                            index.span(),
                            format!("index {constant} is outside `{name}` (length {length})"),
                        );
                    }
                    return Ok(Place::Direct {
                        addr: layout.element(binding.addr, constant as usize),
                        ty: *element,
                    });
                }

                let index_value = self.eval(index)?;
                if !index_value.ty.is_scalar() {
                    return error(index.span(), "array indices must be scalars");
                }
                Ok(Place::Indexed {
                    base: binding.addr,
                    layout,
                    element: *element,
                    index: index_value,
                })
            }
            other => error(other.span(), "this expression cannot be assigned to"),
        }
    }

    fn load(&mut self, place: &Place) -> CResult<Value> {
        match place {
            Place::Direct { addr, ty } => Ok(Value {
                addr: *addr,
                ty: ty.clone(),
            }),
            Place::Indexed {
                base,
                layout,
                element,
                index,
            } => {
                let width = element.width();
                let out = self.bf.alloc_zeroed(width);
                self.array_read(*base, layout, index, out);
                Ok(Value {
                    addr: out,
                    ty: element.clone(),
                })
            }
        }
    }

    fn store(&mut self, place: &Place, value: &Value, span: Span) -> CResult<()> {
        match place {
            Place::Direct { addr, ty } => self.store_scalar(*addr, ty, value, span),
            Place::Indexed {
                base,
                layout,
                element,
                index,
            } => {
                let mark = self.bf.watermark();
                let staged = self.bf.alloc_zeroed(element.width());
                self.store_scalar(staged, element, value, span)?;
                self.array_write(*base, layout, index, staged);
                self.bf.release_to(mark);
                Ok(())
            }
        }
    }

    fn array_read(&mut self, base: Addr, layout: &ArrayLayout, index: &Value, out: Addr) {
        self.bf.num_convert(
            index.addr,
            index.ty.width(),
            layout.counter(base),
            layout.index_width,
        );
        if !layout.flag_is_counter() {
            self.bf
                .num_is_nonzero(base + layout.go(), layout.counter(base), layout.index_width);
        }
        self.bf.array_access(base, layout, Access::Read);
        let transit = layout.transit(base);
        for cell in 0..layout.element_width as Addr {
            self.bf.zero(out + cell);
            self.bf.move_add(transit + cell, &[out + cell]);
        }
    }

    fn array_write(&mut self, base: Addr, layout: &ArrayLayout, index: &Value, value: Addr) {
        self.bf.num_convert(
            index.addr,
            index.ty.width(),
            layout.counter(base),
            layout.index_width,
        );
        if !layout.flag_is_counter() {
            self.bf
                .num_is_nonzero(base + layout.go(), layout.counter(base), layout.index_width);
        }
        self.bf
            .num_copy(value, layout.transit(base), layout.element_width);
        self.bf.array_access(base, layout, Access::Write);
    }

    // ---- expressions ------------------------------------------------------

    fn store_scalar(&mut self, dst: Addr, dst_ty: &Type, value: &Value, span: Span) -> CResult<()> {
        if matches!(dst_ty, Type::Unit) {
            return Ok(());
        }
        if !coercible(&value.ty, dst_ty) {
            let hint = match (&value.ty, dst_ty) {
                (Type::Int, Type::Byte) => " (add `as byte` to truncate)",
                (Type::Byte | Type::Int, Type::Bool) => " (compare with `!= 0`)",
                _ => "",
            };
            return error(
                span,
                format!(
                    "cannot use `{}` where `{dst_ty}` is expected{hint}",
                    value.ty
                ),
            );
        }
        self.bf
            .num_convert(value.addr, value.ty.width(), dst, dst_ty.width());
        Ok(())
    }

    fn truthy(&mut self, value: &Value, out: Addr, span: Span) -> CResult<()> {
        if !value.ty.is_scalar() {
            return error(span, format!("`{}` is not a condition", value.ty));
        }
        self.bf.num_is_nonzero(out, value.addr, value.ty.width());
        Ok(())
    }

    fn eval(&mut self, expr: &'a Expr) -> CResult<Value> {
        match expr {
            Expr::Int { value, span } => {
                let ty = if *value <= 255 { Type::Byte } else { Type::Int };
                if *value > ty.max_value() {
                    return error(*span, format!("{value} does not fit in `int`"));
                }
                let addr = self.bf.alloc(ty.width());
                self.bf.num_set(addr, ty.width(), *value);
                Ok(Value { addr, ty })
            }

            Expr::Bool { value, .. } => {
                let addr = self.bf.alloc(1);
                self.bf.set(addr, u8::from(*value));
                Ok(Value {
                    addr,
                    ty: Type::Bool,
                })
            }

            Expr::Str { span, .. } => error(
                *span,
                "string literals can only initialize arrays or be printed",
            ),

            Expr::ArrayLit { span, .. } => {
                error(*span, "array literals can only initialize arrays")
            }

            Expr::Name { name, span } => {
                if let Some(binding) = self.lookup(name) {
                    return Ok(Value {
                        addr: binding.addr,
                        ty: binding.ty.clone(),
                    });
                }
                if let Some((ty, value)) = self.constants.get(name).cloned() {
                    let addr = self.bf.alloc(ty.width());
                    self.bf.num_set(addr, ty.width(), value);
                    return Ok(Value { addr, ty });
                }
                error(*span, format!("`{name}` is not defined"))
            }

            Expr::Index { .. } => {
                let place = self.place(expr)?;
                self.load(&place)
            }

            Expr::Cast { value, ty, span } => {
                let evaluated = self.eval(value)?;
                if !evaluated.ty.is_scalar() || !ty.is_scalar() {
                    return error(*span, "only scalars can be cast");
                }
                let addr = self.bf.alloc_zeroed(ty.width());
                if matches!(ty, Type::Bool) {
                    self.bf
                        .num_is_nonzero(addr, evaluated.addr, evaluated.ty.width());
                } else {
                    self.bf
                        .num_convert(evaluated.addr, evaluated.ty.width(), addr, ty.width());
                }
                Ok(Value {
                    addr,
                    ty: ty.clone(),
                })
            }

            Expr::Unary { op, operand, span } => {
                let evaluated = self.eval(operand)?;
                match op {
                    UnOp::Not => {
                        let addr = self.bf.alloc_zeroed(1);
                        if !evaluated.ty.is_scalar() {
                            return error(*span, "`!` needs a scalar");
                        }
                        self.bf
                            .num_is_zero(addr, evaluated.addr, evaluated.ty.width());
                        Ok(Value {
                            addr,
                            ty: Type::Bool,
                        })
                    }
                    UnOp::Neg => {
                        let ty = promote(&evaluated.ty);
                        if !ty.is_scalar() {
                            return error(*span, "`-` needs a scalar");
                        }
                        let width = ty.width();
                        let addr = self.bf.alloc_zeroed(width);
                        let staged = self.bf.alloc_zeroed(width);
                        self.bf
                            .num_convert(evaluated.addr, evaluated.ty.width(), staged, width);
                        self.bf.num_sub_assign(addr, staged, width);
                        Ok(Value { addr, ty })
                    }
                }
            }

            Expr::Binary { op, lhs, rhs, span } => {
                if matches!(op, BinOp::And | BinOp::Or) {
                    return self.short_circuit(*op, lhs, rhs, *span);
                }
                let left = self.eval(lhs)?;
                self.binary(*op, &left, rhs, *span)
            }

            Expr::Call { name, args, span } => self.call(name, args, *span),
        }
    }

    /// Apply a binary operator whose left side is already evaluated.
    ///
    /// Compound assignment reuses this so `a[i] += 1` evaluates `i` once.
    fn binary(&mut self, op: BinOp, left: &Value, rhs: &'a Expr, span: Span) -> CResult<Value> {
        let right = self.eval(rhs)?;
        if !left.ty.is_scalar() || !right.ty.is_scalar() {
            return error(span, format!("`{}` needs scalar operands", op.spelling()));
        }

        if matches!(op, BinOp::Shl | BinOp::Shr) {
            let ty = promote(&left.ty);
            let width = ty.width();
            let addr = self.bf.alloc_zeroed(width);
            self.bf.num_convert(left.addr, left.ty.width(), addr, width);
            if let Ok(amount) = self.const_eval(rhs) {
                if matches!(op, BinOp::Shl) {
                    self.bf.num_shl_const(addr, width, amount as usize);
                } else {
                    self.bf.num_shr_const(addr, width, amount as usize);
                }
            } else {
                self.bf.num_shift_dynamic(
                    addr,
                    width,
                    right.addr,
                    right.ty.width(),
                    matches!(op, BinOp::Shl),
                );
            }
            return Ok(Value { addr, ty });
        }

        let operand_ty = wider(&promote(&left.ty), &promote(&right.ty));
        let width = operand_ty.width();
        let staged_left = self.bf.alloc_zeroed(width);
        let staged_right = self.bf.alloc_zeroed(width);
        self.bf
            .num_convert(left.addr, left.ty.width(), staged_left, width);
        self.bf
            .num_convert(right.addr, right.ty.width(), staged_right, width);

        if op.is_comparison() {
            let addr = self.bf.alloc_zeroed(1);
            match op {
                BinOp::Eq => self.bf.num_eq(addr, staged_left, staged_right, width),
                BinOp::Ne => {
                    let same = self.bf.alloc_zeroed(1);
                    self.bf.num_eq(same, staged_left, staged_right, width);
                    self.bf.is_zero(addr, same);
                }
                BinOp::Lt => self.bf.num_lt(addr, staged_left, staged_right, width),
                BinOp::Gt => self.bf.num_lt(addr, staged_right, staged_left, width),
                BinOp::Le => self.bf.num_ge(addr, staged_right, staged_left, width),
                BinOp::Ge => self.bf.num_ge(addr, staged_left, staged_right, width),
                _ => unreachable!("comparison operators are covered above"),
            }
            return Ok(Value {
                addr,
                ty: Type::Bool,
            });
        }

        let addr = self.bf.alloc_zeroed(width);
        match op {
            BinOp::Add => {
                self.bf.num_add_assign(staged_left, staged_right, width);
                self.bf.num_copy(staged_left, addr, width);
            }
            BinOp::Sub => {
                self.bf.num_sub_assign(staged_left, staged_right, width);
                self.bf.num_copy(staged_left, addr, width);
            }
            BinOp::Mul => match self.const_eval(rhs) {
                Ok(factor) => self.bf.num_mul_const(addr, staged_left, width, factor),
                Err(_) => self.bf.num_mul(addr, staged_left, staged_right, width),
            },
            BinOp::Div => {
                let remainder = self.bf.alloc_zeroed(width);
                self.bf
                    .num_divmod(addr, remainder, staged_left, staged_right, width);
            }
            BinOp::Rem => {
                let quotient = self.bf.alloc_zeroed(width);
                self.bf
                    .num_divmod(quotient, addr, staged_left, staged_right, width);
            }
            BinOp::BitAnd => {
                self.bf
                    .num_bitwise(addr, staged_left, staged_right, width, BitKind::And)
            }
            BinOp::BitOr => {
                self.bf
                    .num_bitwise(addr, staged_left, staged_right, width, BitKind::Or)
            }
            BinOp::BitXor => {
                self.bf
                    .num_bitwise(addr, staged_left, staged_right, width, BitKind::Xor)
            }
            _ => unreachable!("remaining operators are handled above"),
        }

        Ok(Value {
            addr,
            ty: operand_ty,
        })
    }

    fn short_circuit(
        &mut self,
        op: BinOp,
        lhs: &'a Expr,
        rhs: &'a Expr,
        span: Span,
    ) -> CResult<Value> {
        let out = self.bf.alloc_zeroed(1);
        let left_flag = self.bf.alloc_zeroed(1);
        let mark = self.bf.watermark();
        let left = self.eval(lhs)?;
        self.truthy(&left, left_flag, span)?;
        self.bf.release_to(mark);

        let wants_left = matches!(op, BinOp::And);
        if !wants_left {
            self.bf.set(out, 1);
        }

        // For `&&` the right side only runs when the left was true; for `||`
        // only when it was false.
        let gate = self.bf.alloc_zeroed(1);
        if wants_left {
            self.bf.add_copy(left_flag, gate);
        } else {
            self.bf.is_zero(gate, left_flag);
        }

        self.bf.open_loop(gate);
        let inner = self.bf.watermark();
        let right = self.eval(rhs)?;
        let right_flag = self.bf.alloc_zeroed(1);
        self.truthy(&right, right_flag, span)?;
        if wants_left {
            self.bf.if_nonzero_consume(right_flag, |bf| bf.set(out, 1));
        } else {
            self.bf.if_zero(right_flag, |bf| bf.zero(out));
            self.bf.zero(right_flag);
        }
        self.bf.release_to(inner);
        self.bf.zero(gate);
        self.bf.close_loop(gate);

        Ok(Value {
            addr: out,
            ty: Type::Bool,
        })
    }

    // ---- calls ------------------------------------------------------------

    fn call(&mut self, name: &str, args: &'a [Expr], span: Span) -> CResult<Value> {
        match name {
            "print" => return self.builtin_print(args, false),
            "println" => return self.builtin_print(args, true),
            "putc" => return self.builtin_putc(args, span),
            "getc" => return self.builtin_getc(args, span),
            "puts" => return self.builtin_puts(args, span),
            "len" => return self.builtin_len(args, span),
            _ => {}
        }

        let Some(function) = self.functions.get(name).copied() else {
            return error(span, format!("`{name}` is not defined"));
        };
        self.inline_call(function, args, span)
    }

    fn inline_call(
        &mut self,
        function: &'a Function,
        args: &'a [Expr],
        span: Span,
    ) -> CResult<Value> {
        if self.inlining.iter().any(|name| name == &function.name) {
            return error(
                span,
                format!(
                    "`{}` calls itself; Brainfuck has no call stack, so Cranium inlines every call and cannot compile recursion",
                    function.name
                ),
            );
        }
        if args.len() != function.params.len() {
            return error(
                span,
                format!(
                    "`{}` takes {} argument(s) but {} were given",
                    function.name,
                    function.params.len(),
                    args.len()
                ),
            );
        }

        let return_slot = self.bf.alloc_zeroed(function.ret.width());
        let mark = self.bf.watermark();

        let mut scope = Scope {
            names: HashMap::new(),
            barrier: true,
        };
        for (param, arg) in function.params.iter().zip(args) {
            let binding = self.bind_argument(param, arg)?;
            scope.names.insert(param.name.clone(), binding);
        }

        self.scopes.push(scope);
        self.inlining.push(function.name.clone());
        self.returns.push((return_slot, function.ret.clone()));
        let outer_loops = std::mem::take(&mut self.loop_depth);

        let result = self.statements(&function.body);

        self.loop_depth = outer_loops;
        self.returns.pop();
        self.inlining.pop();
        self.pop_scope();
        result?;

        self.clear_control(CONTROL_RETURN);
        self.bf.release_to(mark);

        Ok(Value {
            addr: return_slot,
            ty: function.ret.clone(),
        })
    }

    fn bind_argument(&mut self, param: &Param, arg: &'a Expr) -> CResult<Binding> {
        if let Type::Array { .. } = &param.ty {
            // Arrays are passed by reference: the callee shares the caller's
            // cells rather than copying a whole region.
            let Expr::Name { name, span } = arg else {
                return error(arg.span(), "array arguments must be named variables");
            };
            let Some(binding) = self.lookup(name).cloned() else {
                return error(*span, format!("`{name}` is not defined"));
            };
            if binding.ty != param.ty {
                return error(
                    *span,
                    format!(
                        "`{name}` has type `{}` but `{}` is expected",
                        binding.ty, param.ty
                    ),
                );
            }
            return Ok(binding);
        }

        let addr = self.bf.alloc_zeroed(param.ty.width());
        let mark = self.bf.watermark();
        let value = self.eval(arg)?;
        self.store_scalar(addr, &param.ty, &value, arg.span())?;
        self.bf.release_to(mark);
        Ok(Binding {
            addr,
            ty: param.ty.clone(),
        })
    }

    // ---- builtins ---------------------------------------------------------

    fn builtin_print(&mut self, args: &'a [Expr], newline: bool) -> CResult<Value> {
        for arg in args {
            let mark = self.bf.watermark();
            match arg {
                Expr::Str { value, .. } => {
                    let scratch = self.bf.alloc_zeroed(1);
                    self.bf.write_bytes(scratch, value);
                }
                other => {
                    let value = self.eval(other)?;
                    match &value.ty {
                        Type::Byte | Type::Int | Type::Bool => {
                            self.bf.num_print_decimal(value.addr, value.ty.width());
                        }
                        Type::Array { element, length } if matches!(**element, Type::Byte) => {
                            let layout = ArrayLayout::new(*length, 1);
                            self.emit_puts(value.addr, &layout);
                        }
                        other_ty => {
                            return error(other.span(), format!("`{other_ty}` cannot be printed"))
                        }
                    }
                }
            }
            self.bf.release_to(mark);
        }

        if newline {
            let mark = self.bf.watermark();
            let scratch = self.bf.alloc_zeroed(1);
            self.bf.write_literal(scratch, b'\n');
            self.bf.zero(scratch);
            self.bf.release_to(mark);
        }

        Ok(Value {
            addr: 0,
            ty: Type::Unit,
        })
    }

    fn builtin_putc(&mut self, args: &'a [Expr], span: Span) -> CResult<Value> {
        let [arg] = args else {
            return error(span, "`putc` takes exactly one argument");
        };
        let mark = self.bf.watermark();
        let value = self.eval(arg)?;
        if !value.ty.is_scalar() {
            return error(arg.span(), "`putc` needs a scalar");
        }
        self.bf.write(value.addr);
        self.bf.release_to(mark);
        Ok(Value {
            addr: 0,
            ty: Type::Unit,
        })
    }

    fn builtin_getc(&mut self, args: &'a [Expr], span: Span) -> CResult<Value> {
        if !args.is_empty() {
            return error(span, "`getc` takes no arguments");
        }
        let addr = self.bf.alloc(1);
        self.bf.read(addr);
        Ok(Value {
            addr,
            ty: Type::Byte,
        })
    }

    fn builtin_puts(&mut self, args: &'a [Expr], span: Span) -> CResult<Value> {
        let [arg] = args else {
            return error(span, "`puts` takes exactly one array");
        };
        let mark = self.bf.watermark();
        let value = self.eval(arg)?;
        let Type::Array { element, length } = &value.ty else {
            return error(arg.span(), "`puts` needs a `byte` array");
        };
        if !matches!(**element, Type::Byte) {
            return error(arg.span(), "`puts` needs a `byte` array");
        }
        let layout = ArrayLayout::new(*length, 1);
        self.emit_puts(value.addr, &layout);
        self.bf.release_to(mark);
        Ok(Value {
            addr: 0,
            ty: Type::Unit,
        })
    }

    /// Print a byte array up to its first `0`, or to the end.
    fn emit_puts(&mut self, base: Addr, layout: &ArrayLayout) {
        let mark = self.bf.watermark();
        let index = self.bf.alloc_zeroed(layout.index_width);
        let running = self.bf.alloc_zeroed(1);
        let byte = self.bf.alloc_zeroed(1);
        let limit = self.bf.alloc_zeroed(layout.index_width);
        self.bf
            .num_set(limit, layout.index_width, layout.length as u64);
        self.bf.set(running, 1);

        self.bf.open_loop(running);
        let cursor = Value {
            addr: index,
            ty: if layout.index_width == 1 {
                Type::Byte
            } else {
                Type::Int
            },
        };
        self.array_read(base, layout, &cursor, byte);
        self.bf.if_else(
            byte,
            |bf| {
                bf.write(byte);
                bf.zero(byte);
            },
            |bf| bf.zero(running),
        );
        self.increment(index, layout.index_width);
        let reached_end = self.bf.alloc_zeroed(1);
        self.bf
            .num_eq(reached_end, index, limit, layout.index_width);
        self.bf
            .if_nonzero_consume(reached_end, |bf| bf.zero(running));
        self.bf.close_loop(running);

        self.bf.zero(byte);
        self.bf.num_zero(index, layout.index_width);
        self.bf.num_zero(limit, layout.index_width);
        self.bf.release_to(mark);
    }

    fn builtin_len(&mut self, args: &'a [Expr], span: Span) -> CResult<Value> {
        let [arg] = args else {
            return error(span, "`len` takes exactly one array");
        };
        let ty = self.type_of(arg)?;
        let Type::Array { length, .. } = ty else {
            return error(arg.span(), "`len` needs an array");
        };
        let addr = self.bf.alloc(2);
        self.bf.num_set(addr, 2, length as u64);
        Ok(Value {
            addr,
            ty: Type::Int,
        })
    }
}

/// `bool` behaves like a `byte` once it takes part in arithmetic.
fn promote(ty: &Type) -> Type {
    match ty {
        Type::Bool => Type::Byte,
        other => other.clone(),
    }
}

fn wider(lhs: &Type, rhs: &Type) -> Type {
    if matches!(lhs, Type::Int) || matches!(rhs, Type::Int) {
        Type::Int
    } else if lhs.is_scalar() && rhs.is_scalar() {
        Type::Byte
    } else {
        lhs.clone()
    }
}

fn coercible(from: &Type, to: &Type) -> bool {
    if from == to {
        return true;
    }
    matches!(
        (from, to),
        (Type::Bool, Type::Byte | Type::Int) | (Type::Byte, Type::Int)
    )
}

/// True when a statement can leave the control cell set for its own block.
fn interrupts(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Return { .. } => true,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            then_block.iter().any(interrupts)
                || else_block
                    .as_ref()
                    .is_some_and(|body| body.iter().any(interrupts))
        }
        Stmt::Block { body, .. } => body.iter().any(interrupts),
        // A loop absorbs its own `break` and `continue`; only `return` escapes.
        Stmt::While { body, .. } | Stmt::Loop { body, .. } | Stmt::For { body, .. } => {
            body.iter().any(returns)
        }
        _ => false,
    }
}

fn returns(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return { .. } => true,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            then_block.iter().any(returns)
                || else_block
                    .as_ref()
                    .is_some_and(|body| body.iter().any(returns))
        }
        Stmt::Block { body, .. }
        | Stmt::While { body, .. }
        | Stmt::Loop { body, .. }
        | Stmt::For { body, .. } => body.iter().any(returns),
        _ => false,
    }
}
