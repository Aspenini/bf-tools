//! Recursive-descent parser turning tokens into a [`Program`].

use crate::ast::{BinOp, Expr, Function, Item, Param, Program, Stmt, Type, UnOp};
use crate::lexer::{Span, Tok, Token};
use std::fmt;

/// A parse failure.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    /// Human readable description.
    pub message: String,
    /// Where the problem was found.
    pub span: Span,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.span, self.message)
    }
}

type PResult<T> = Result<T, ParseError>;

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

/// Parse a token stream into a program.
pub fn parse(tokens: Vec<Token>) -> PResult<Program> {
    let mut parser = Parser { tokens, index: 0 };
    parser.program()
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.tokens[self.index].tok
    }

    fn span(&self) -> Span {
        self.tokens[self.index].span
    }

    fn advance(&mut self) -> Token {
        if self.index + 1 >= self.tokens.len() {
            return self.tokens[self.index].clone();
        }
        let span = self.tokens[self.index].span;
        let token = std::mem::replace(
            &mut self.tokens[self.index],
            Token {
                tok: Tok::Eof,
                span,
            },
        );
        self.index += 1;
        token
    }

    fn take_ident(&mut self) -> String {
        match self.advance().tok {
            Tok::Ident(name) => name,
            _ => unreachable!("caller peeked an identifier"),
        }
    }

    fn take_str(&mut self) -> Vec<u8> {
        match self.advance().tok {
            Tok::Str(value) => value,
            _ => unreachable!("caller peeked a string"),
        }
    }

    fn error<T>(&self, message: impl Into<String>) -> PResult<T> {
        Err(ParseError {
            message: message.into(),
            span: self.span(),
        })
    }

    fn at_punct(&self, text: &str) -> bool {
        matches!(self.peek(), Tok::Punct(found) if *found == text)
    }

    fn at_keyword(&self, text: &str) -> bool {
        matches!(self.peek(), Tok::Ident(found) if found == text)
    }

    fn eat_punct(&mut self, text: &str) -> bool {
        if self.at_punct(text) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn eat_keyword(&mut self, text: &str) -> bool {
        if self.at_keyword(text) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, text: &str) -> PResult<Span> {
        let span = self.span();
        if self.eat_punct(text) {
            Ok(span)
        } else {
            self.error(format!("expected `{text}`, found {}", self.peek()))
        }
    }

    fn expect_ident(&mut self) -> PResult<String> {
        match self.peek() {
            Tok::Ident(name) if !is_keyword(name) => Ok(self.take_ident()),
            Tok::Ident(name) => self.error(format!("`{name}` is a keyword and cannot be a name")),
            other => self.error(format!("expected an identifier, found {other}")),
        }
    }

    fn program(&mut self) -> PResult<Program> {
        let mut items = Vec::new();
        while !matches!(self.peek(), Tok::Eof) {
            items.push(self.item()?);
        }
        Ok(Program { items })
    }

    fn item(&mut self) -> PResult<Item> {
        let span = self.span();
        if self.eat_keyword("import") {
            let Tok::Str(_) = self.peek() else {
                return self.error("`import` needs a quoted path, as in `import \"lib.cra\";`");
            };
            let Ok(path) = String::from_utf8(self.take_str()) else {
                return self.error("import paths must be text");
            };
            self.expect_punct(";")?;
            return Ok(Item::Import { path, span });
        }
        if self.eat_keyword("fn") {
            return Ok(Item::Function(self.function(span)?));
        }
        if self.eat_keyword("const") {
            let name = self.expect_ident()?;
            let ty = if self.eat_punct(":") {
                Some(self.ty()?)
            } else {
                None
            };
            self.expect_punct("=")?;
            let value = self.expr()?;
            self.expect_punct(";")?;
            return Ok(Item::Const {
                name,
                ty,
                value,
                span,
            });
        }
        if self.eat_keyword("let") {
            let name = self.expect_ident()?;
            let ty = if self.eat_punct(":") {
                Some(self.ty()?)
            } else {
                None
            };
            let init = if self.eat_punct("=") {
                Some(self.expr()?)
            } else {
                None
            };
            self.expect_punct(";")?;
            return Ok(Item::Global {
                name,
                ty,
                init,
                span,
            });
        }
        self.error(format!(
            "expected `import`, `fn`, `const`, or `let` at the top level, found {}",
            self.peek()
        ))
    }

    fn function(&mut self, span: Span) -> PResult<Function> {
        let name = self.expect_ident()?;
        self.expect_punct("(")?;
        let mut params = Vec::new();
        if !self.at_punct(")") {
            loop {
                let param_span = self.span();
                let param_name = self.expect_ident()?;
                self.expect_punct(":")?;
                let ty = self.ty()?;
                params.push(Param {
                    name: param_name,
                    ty,
                    span: param_span,
                });
                if !self.eat_punct(",") {
                    break;
                }
            }
        }
        self.expect_punct(")")?;
        let ret = if self.eat_punct("->") {
            self.ty()?
        } else {
            Type::Unit
        };
        let body = self.block()?;
        Ok(Function {
            name,
            params,
            ret,
            body,
            span,
        })
    }

    fn ty(&mut self) -> PResult<Type> {
        let mut ty = match self.peek() {
            Tok::Ident(_) => match self.take_ident().as_str() {
                "byte" => Type::Byte,
                "int" => Type::Int,
                "sbyte" => Type::SByte,
                "sint" => Type::SInt,
                "bool" => Type::Bool,
                other => return self.error(format!("unknown type `{other}`")),
            },
            other => return self.error(format!("expected a type, found {other}")),
        };

        while self.at_punct("[") {
            self.advance();
            let length = match self.peek() {
                Tok::Int(value) => {
                    let value = *value;
                    self.advance();
                    let Ok(length) = usize::try_from(value) else {
                        return self.error("array length is too large");
                    };
                    length
                }
                other => return self.error(format!("expected an array length, found {other}")),
            };
            self.expect_punct("]")?;
            if matches!(ty, Type::Array { .. }) {
                return self.error("nested array types are not supported");
            }
            if length == 0 {
                return self.error("array length must be at least 1");
            }
            ty = Type::Array {
                element: Box::new(ty),
                length,
            };
        }

        Ok(ty)
    }

    fn block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect_punct("{")?;
        let mut body = Vec::new();
        while !self.at_punct("}") {
            if matches!(self.peek(), Tok::Eof) {
                return self.error("unexpected end of file inside a block");
            }
            body.push(self.stmt()?);
        }
        self.expect_punct("}")?;
        Ok(body)
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let span = self.span();

        if self.at_punct("{") {
            return Ok(Stmt::Block {
                body: self.block()?,
                span,
            });
        }

        if self.eat_keyword("let") {
            let name = self.expect_ident()?;
            let ty = if self.eat_punct(":") {
                Some(self.ty()?)
            } else {
                None
            };
            let init = if self.eat_punct("=") {
                Some(self.expr()?)
            } else {
                None
            };
            self.expect_punct(";")?;
            return Ok(Stmt::Let {
                name,
                ty,
                init,
                span,
            });
        }

        if self.eat_keyword("if") {
            return self.if_stmt(span);
        }

        if self.eat_keyword("while") {
            let cond = self.expr()?;
            let body = self.block()?;
            return Ok(Stmt::While { cond, body, span });
        }

        if self.eat_keyword("loop") {
            let body = self.block()?;
            return Ok(Stmt::Loop { body, span });
        }

        if self.eat_keyword("for") {
            let name = self.expect_ident()?;
            if !self.eat_keyword("in") {
                return self.error("expected `in` after the loop variable");
            }
            let start = self.expr()?;
            self.expect_punct("..")?;
            let end = self.expr()?;
            let body = self.block()?;
            return Ok(Stmt::For {
                name,
                start,
                end,
                body,
                span,
            });
        }

        if self.eat_keyword("break") {
            self.expect_punct(";")?;
            return Ok(Stmt::Break { span });
        }

        if self.eat_keyword("continue") {
            self.expect_punct(";")?;
            return Ok(Stmt::Continue { span });
        }

        if self.eat_keyword("return") {
            let value = if self.at_punct(";") {
                None
            } else {
                Some(self.expr()?)
            };
            self.expect_punct(";")?;
            return Ok(Stmt::Return { value, span });
        }

        let expr = self.expr()?;
        let op_span = self.span();
        let compound = match self.peek() {
            Tok::Punct("=") => Some(None),
            Tok::Punct("+=") => Some(Some(BinOp::Add)),
            Tok::Punct("-=") => Some(Some(BinOp::Sub)),
            Tok::Punct("*=") => Some(Some(BinOp::Mul)),
            Tok::Punct("/=") => Some(Some(BinOp::Div)),
            Tok::Punct("%=") => Some(Some(BinOp::Rem)),
            _ => None,
        };

        if let Some(op) = compound {
            self.advance();
            let value = self.expr()?;
            self.expect_punct(";")?;
            return Ok(Stmt::Assign {
                target: expr,
                op,
                value,
                span: op_span,
            });
        }

        self.expect_punct(";")?;
        Ok(Stmt::Expr { expr, span })
    }

    fn if_stmt(&mut self, span: Span) -> PResult<Stmt> {
        let cond = self.expr()?;
        let then_block = self.block()?;
        let else_block = if self.eat_keyword("else") {
            if self.at_keyword("if") {
                let else_span = self.span();
                self.advance();
                Some(vec![self.if_stmt(else_span)?])
            } else {
                Some(self.block()?)
            }
        } else {
            None
        };
        Ok(Stmt::If {
            cond,
            then_block,
            else_block,
            span,
        })
    }

    fn expr(&mut self) -> PResult<Expr> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.and_expr()?;
        while self.at_punct("||") {
            let span = self.span();
            self.advance();
            let rhs = self.and_expr()?;
            lhs = Expr::Binary {
                op: BinOp::Or,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.cmp_expr()?;
        while self.at_punct("&&") {
            let span = self.span();
            self.advance();
            let rhs = self.cmp_expr()?;
            lhs = Expr::Binary {
                op: BinOp::And,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn cmp_expr(&mut self) -> PResult<Expr> {
        let lhs = self.bitor_expr()?;
        let op = match self.peek() {
            Tok::Punct("==") => BinOp::Eq,
            Tok::Punct("!=") => BinOp::Ne,
            Tok::Punct("<") => BinOp::Lt,
            Tok::Punct("<=") => BinOp::Le,
            Tok::Punct(">") => BinOp::Gt,
            Tok::Punct(">=") => BinOp::Ge,
            _ => return Ok(lhs),
        };
        let span = self.span();
        self.advance();
        let rhs = self.bitor_expr()?;
        Ok(Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        })
    }

    fn bitor_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.bitand_expr()?;
        loop {
            let op = match self.peek() {
                Tok::Punct("|") => BinOp::BitOr,
                Tok::Punct("^") => BinOp::BitXor,
                _ => return Ok(lhs),
            };
            let span = self.span();
            self.advance();
            let rhs = self.bitand_expr()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
    }

    fn bitand_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.shift_expr()?;
        while self.at_punct("&") {
            let span = self.span();
            self.advance();
            let rhs = self.shift_expr()?;
            lhs = Expr::Binary {
                op: BinOp::BitAnd,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn shift_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.add_expr()?;
        loop {
            let op = match self.peek() {
                Tok::Punct("<<") => BinOp::Shl,
                Tok::Punct(">>") => BinOp::Shr,
                _ => return Ok(lhs),
            };
            let span = self.span();
            self.advance();
            let rhs = self.add_expr()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
    }

    fn add_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.mul_expr()?;
        loop {
            let op = match self.peek() {
                Tok::Punct("+") => BinOp::Add,
                Tok::Punct("-") => BinOp::Sub,
                _ => return Ok(lhs),
            };
            let span = self.span();
            self.advance();
            let rhs = self.mul_expr()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
    }

    fn mul_expr(&mut self) -> PResult<Expr> {
        let mut lhs = self.cast_expr()?;
        loop {
            let op = match self.peek() {
                Tok::Punct("*") => BinOp::Mul,
                Tok::Punct("/") => BinOp::Div,
                Tok::Punct("%") => BinOp::Rem,
                _ => return Ok(lhs),
            };
            let span = self.span();
            self.advance();
            let rhs = self.cast_expr()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
    }

    fn cast_expr(&mut self) -> PResult<Expr> {
        let mut value = self.unary_expr()?;
        while self.at_keyword("as") {
            let span = self.span();
            self.advance();
            let ty = self.ty()?;
            value = Expr::Cast {
                value: Box::new(value),
                ty,
                span,
            };
        }
        Ok(value)
    }

    fn unary_expr(&mut self) -> PResult<Expr> {
        let span = self.span();
        let op = match self.peek() {
            Tok::Punct("!") => UnOp::Not,
            Tok::Punct("-") => UnOp::Neg,
            _ => return self.postfix_expr(),
        };
        self.advance();
        let operand = self.unary_expr()?;
        Ok(Expr::Unary {
            op,
            operand: Box::new(operand),
            span,
        })
    }

    fn postfix_expr(&mut self) -> PResult<Expr> {
        let mut value = self.primary_expr()?;
        while self.at_punct("[") {
            let span = self.span();
            self.advance();
            let index = self.expr()?;
            self.expect_punct("]")?;
            value = Expr::Index {
                array: Box::new(value),
                index: Box::new(index),
                span,
            };
        }
        Ok(value)
    }

    fn primary_expr(&mut self) -> PResult<Expr> {
        let span = self.span();
        match self.peek() {
            Tok::Int(value) => {
                let value = *value;
                self.advance();
                Ok(Expr::Int { value, span })
            }
            Tok::Str(_) => Ok(Expr::Str {
                value: self.take_str(),
                span,
            }),
            Tok::Punct("(") => {
                self.advance();
                let inner = self.expr()?;
                self.expect_punct(")")?;
                Ok(inner)
            }
            Tok::Punct("[") => {
                self.advance();
                let mut elements = Vec::new();
                if !self.at_punct("]") {
                    loop {
                        elements.push(self.expr()?);
                        if !self.eat_punct(",") {
                            break;
                        }
                    }
                }
                self.expect_punct("]")?;
                Ok(Expr::ArrayLit { elements, span })
            }
            Tok::Ident(name) if name == "true" || name == "false" => {
                let value = name == "true";
                self.advance();
                Ok(Expr::Bool { value, span })
            }
            Tok::Ident(name) if is_keyword(name) => {
                self.error(format!("`{name}` cannot start an expression"))
            }
            Tok::Ident(_) => {
                let name = self.take_ident();
                if self.eat_punct("(") {
                    let mut args = Vec::new();
                    if !self.at_punct(")") {
                        loop {
                            args.push(self.expr()?);
                            if !self.eat_punct(",") {
                                break;
                            }
                        }
                    }
                    self.expect_punct(")")?;
                    return Ok(Expr::Call { name, args, span });
                }
                Ok(Expr::Name { name, span })
            }
            other => self.error(format!("expected an expression, found {other}")),
        }
    }
}

fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "fn" | "let"
            | "const"
            | "import"
            | "if"
            | "else"
            | "while"
            | "for"
            | "in"
            | "loop"
            | "break"
            | "continue"
            | "return"
            | "as"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    fn parse_src(src: &str) -> PResult<Program> {
        parse(tokenize(src, 0).expect("valid tokens"))
    }

    #[test]
    fn parses_a_function() {
        let program = parse_src("fn main() { let x: byte = 1; }").expect("valid program");
        let Item::Function(function) = &program.items[0] else {
            panic!("expected a function");
        };
        assert_eq!(function.name, "main");
        assert_eq!(function.ret, Type::Unit);
        assert_eq!(function.body.len(), 1);
    }

    #[test]
    fn respects_precedence() {
        let program = parse_src("fn main() { let x = 1 + 2 * 3; }").expect("valid program");
        let Item::Function(function) = &program.items[0] else {
            panic!("expected a function");
        };
        let Stmt::Let {
            init: Some(init), ..
        } = &function.body[0]
        else {
            panic!("expected a let with an initializer");
        };
        let Expr::Binary { op, rhs, .. } = init else {
            panic!("expected a binary expression");
        };
        assert_eq!(*op, BinOp::Add);
        assert!(matches!(**rhs, Expr::Binary { op: BinOp::Mul, .. }));
    }

    #[test]
    fn parses_else_if_chains() {
        let program =
            parse_src("fn main() { if 1 { } else if 2 { } else { } }").expect("valid program");
        let Item::Function(function) = &program.items[0] else {
            panic!("expected a function");
        };
        let Stmt::If {
            else_block: Some(chain),
            ..
        } = &function.body[0]
        else {
            panic!("expected an if with an else");
        };
        assert!(matches!(chain[0], Stmt::If { .. }));
    }

    #[test]
    fn rejects_nested_array_types() {
        let err = parse_src("fn main() { let g: byte[2][2]; }").expect_err("nested arrays");
        assert!(err.message.contains("nested array"));
    }

    #[test]
    fn parses_compound_assignment() {
        let program = parse_src("fn main() { let x = 0; x += 2; }").expect("valid program");
        let Item::Function(function) = &program.items[0] else {
            panic!("expected a function");
        };
        assert!(matches!(
            function.body[1],
            Stmt::Assign {
                op: Some(BinOp::Add),
                ..
            }
        ));
    }
}
