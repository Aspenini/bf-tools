//! Abstract syntax tree produced by [`crate::parser`].

use crate::lexer::Span;
use std::fmt;

/// A source-level type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    /// Unsigned 8-bit integer stored in one tape cell.
    Byte,
    /// Unsigned 16-bit integer stored in two little-endian tape cells.
    Int,
    /// Signed 8-bit integer, two's complement, in one tape cell.
    SByte,
    /// Signed 16-bit integer, two's complement, in two little-endian cells.
    SInt,
    /// A byte restricted to `0` or `1`.
    Bool,
    /// Fixed-length array of `element` values.
    Array {
        /// Element type. Arrays of arrays are not supported.
        element: Box<Type>,
        /// Number of elements.
        length: usize,
    },
    /// The type of an expression that produces no value.
    Unit,
}

impl Type {
    /// Number of tape cells one value of this type occupies.
    ///
    /// Array cells are counted for the payload only; [`crate::codegen`] adds the
    /// bookkeeping lanes needed for dynamic indexing.
    pub fn width(&self) -> usize {
        match self {
            Type::Byte | Type::Bool | Type::SByte => 1,
            Type::Int | Type::SInt => 2,
            Type::Array { element, length } => element.width() * length,
            Type::Unit => 0,
        }
    }

    /// True for types that arithmetic and comparison operators accept.
    pub fn is_scalar(&self) -> bool {
        matches!(
            self,
            Type::Byte | Type::Int | Type::SByte | Type::SInt | Type::Bool
        )
    }

    /// True for two's complement types, whose top bit is a sign.
    pub fn is_signed(&self) -> bool {
        matches!(self, Type::SByte | Type::SInt)
    }

    /// Largest value representable, for scalars.
    pub fn max_value(&self) -> i64 {
        match self {
            Type::Bool => 1,
            Type::Byte => 255,
            Type::Int => 65_535,
            Type::SByte => 127,
            Type::SInt => 32_767,
            _ => 0,
        }
    }

    /// Smallest value representable, for scalars.
    pub fn min_value(&self) -> i64 {
        match self {
            Type::SByte => -128,
            Type::SInt => -32_768,
            _ => 0,
        }
    }

    /// Mask covering every bit a value of this type occupies.
    pub fn mask(&self) -> u64 {
        match self.width() {
            0 => 0,
            width => u64::MAX >> (64 - 8 * width as u32),
        }
    }

    /// True when every value of this type also fits in `other`.
    pub fn fits_in(&self, other: &Type) -> bool {
        self.is_scalar()
            && other.is_scalar()
            && other.min_value() <= self.min_value()
            && self.max_value() <= other.max_value()
    }

    /// The signed type that holds this one's values, used by unary `-`.
    pub fn signed_form(&self) -> Type {
        match self {
            Type::Bool | Type::Byte | Type::SByte => Type::SByte,
            Type::Int | Type::SInt => Type::SInt,
            other => other.clone(),
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Byte => write!(f, "byte"),
            Type::Int => write!(f, "int"),
            Type::SByte => write!(f, "sbyte"),
            Type::SInt => write!(f, "sint"),
            Type::Bool => write!(f, "bool"),
            Type::Unit => write!(f, "()"),
            Type::Array { element, length } => write!(f, "{element}[{length}]"),
        }
    }
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `&&`, short-circuiting.
    And,
    /// `||`, short-circuiting.
    Or,
    /// `&`
    BitAnd,
    /// `|`
    BitOr,
    /// `^`
    BitXor,
    /// `<<`
    Shl,
    /// `>>`
    Shr,
}

impl BinOp {
    /// True when the operator yields a `bool`.
    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        )
    }

    /// The operator's source spelling.
    pub fn spelling(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
        }
    }
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `!`, logical negation producing a `bool`.
    Not,
    /// `-`, wrapping negation.
    Neg,
}

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Integer, character, `true`, or `false` literal.
    Int {
        /// The literal value.
        value: u64,
        /// Where it appeared.
        span: Span,
    },
    /// `true` or `false`.
    Bool {
        /// The literal value.
        value: bool,
        /// Where it appeared.
        span: Span,
    },
    /// `[a, b, c]`, only valid as an array initializer.
    ArrayLit {
        /// Element expressions, all of which must be compile-time constants.
        elements: Vec<Expr>,
        /// Where the `[` appeared.
        span: Span,
    },
    /// String literal, only valid as an array initializer or `print` argument.
    Str {
        /// Literal bytes without a terminator.
        value: Vec<u8>,
        /// Where it appeared.
        span: Span,
    },
    /// Variable, constant, or parameter reference.
    Name {
        /// The identifier.
        name: String,
        /// Where it appeared.
        span: Span,
    },
    /// `array[index]`
    Index {
        /// The array being indexed.
        array: Box<Expr>,
        /// The index expression.
        index: Box<Expr>,
        /// Where the `[` appeared.
        span: Span,
    },
    /// A call to a user function or builtin.
    Call {
        /// Callee name.
        name: String,
        /// Argument expressions.
        args: Vec<Expr>,
        /// Where the call appeared.
        span: Span,
    },
    /// Binary operation.
    Binary {
        /// The operator.
        op: BinOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
        /// Where the operator appeared.
        span: Span,
    },
    /// Unary operation.
    Unary {
        /// The operator.
        op: UnOp,
        /// Operand.
        operand: Box<Expr>,
        /// Where the operator appeared.
        span: Span,
    },
    /// `expr as type`
    Cast {
        /// Value being converted.
        value: Box<Expr>,
        /// Destination type.
        ty: Type,
        /// Where the `as` appeared.
        span: Span,
    },
}

impl Expr {
    /// Where this expression starts.
    pub fn span(&self) -> Span {
        match self {
            Expr::Int { span, .. }
            | Expr::Bool { span, .. }
            | Expr::ArrayLit { span, .. }
            | Expr::Str { span, .. }
            | Expr::Name { span, .. }
            | Expr::Index { span, .. }
            | Expr::Call { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Cast { span, .. } => *span,
        }
    }
}

/// A statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `let name: type = init;`
    Let {
        /// Variable name.
        name: String,
        /// Declared type, or `None` to infer from `init`.
        ty: Option<Type>,
        /// Initializer, or `None` to zero-initialize.
        init: Option<Expr>,
        /// Where the `let` appeared.
        span: Span,
    },
    /// `target op= value;` where `op` is `None` for a plain assignment.
    Assign {
        /// Assignment destination.
        target: Expr,
        /// Compound-assignment operator, if any.
        op: Option<BinOp>,
        /// Value to store.
        value: Expr,
        /// Where the operator appeared.
        span: Span,
    },
    /// `if cond { .. } else { .. }`
    If {
        /// Condition.
        cond: Expr,
        /// Taken when `cond` is nonzero.
        then_block: Vec<Stmt>,
        /// Taken otherwise, if present.
        else_block: Option<Vec<Stmt>>,
        /// Where the `if` appeared.
        span: Span,
    },
    /// `while cond { .. }`
    While {
        /// Condition, re-evaluated each iteration.
        cond: Expr,
        /// Loop body.
        body: Vec<Stmt>,
        /// Where the `while` appeared.
        span: Span,
    },
    /// `for name in start..end { .. }`, with `end` exclusive.
    For {
        /// Induction variable name.
        name: String,
        /// Inclusive lower bound.
        start: Expr,
        /// Exclusive upper bound.
        end: Expr,
        /// Loop body.
        body: Vec<Stmt>,
        /// Where the `for` appeared.
        span: Span,
    },
    /// `loop { .. }`, exited with `break`.
    Loop {
        /// Loop body.
        body: Vec<Stmt>,
        /// Where the `loop` appeared.
        span: Span,
    },
    /// `break;`
    Break {
        /// Where it appeared.
        span: Span,
    },
    /// `continue;`
    Continue {
        /// Where it appeared.
        span: Span,
    },
    /// `return expr;`
    Return {
        /// Returned value, if any.
        value: Option<Expr>,
        /// Where it appeared.
        span: Span,
    },
    /// An expression evaluated for its side effects.
    Expr {
        /// The expression.
        expr: Expr,
        /// Where it appeared.
        span: Span,
    },
    /// A nested `{ .. }` scope.
    Block {
        /// Statements in the scope.
        body: Vec<Stmt>,
        /// Where the `{` appeared.
        span: Span,
    },
}

/// A function parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    /// Parameter name.
    pub name: String,
    /// Parameter type.
    pub ty: Type,
    /// Where the parameter appeared.
    pub span: Span,
}

/// A function definition.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    /// Function name.
    pub name: String,
    /// Declared parameters.
    pub params: Vec<Param>,
    /// Return type; [`Type::Unit`] when the function returns nothing.
    pub ret: Type,
    /// Function body.
    pub body: Vec<Stmt>,
    /// Where the `fn` appeared.
    pub span: Span,
}

/// A top-level item.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// `import "path";`, replaced by that file's items before compiling.
    Import {
        /// Path as written, relative to the file the import appears in.
        path: String,
        /// Where the `import` appeared.
        span: Span,
    },
    /// A function definition.
    Function(Function),
    /// A compile-time constant.
    Const {
        /// Constant name.
        name: String,
        /// Declared type, or `None` to infer.
        ty: Option<Type>,
        /// Value expression, evaluated at compile time.
        value: Expr,
        /// Where the `const` appeared.
        span: Span,
    },
    /// A global variable, laid out once for the whole program.
    Global {
        /// Variable name.
        name: String,
        /// Declared type, or `None` to infer from `init`.
        ty: Option<Type>,
        /// Initializer, or `None` to zero-initialize.
        init: Option<Expr>,
        /// Where the `let` appeared.
        span: Span,
    },
}

/// A parsed source file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Program {
    /// Top-level items in source order.
    pub items: Vec<Item>,
}
