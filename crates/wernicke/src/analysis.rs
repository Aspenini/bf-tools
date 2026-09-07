//! What the server knows about a Cranium project.
//!
//! There is no separate model of the language here: diagnostics come from
//! actually compiling, and everything else is read off the same syntax tree the
//! compiler uses. Unsaved buffers reach the compiler through a
//! [`cranium::Loader`] that answers from memory before touching disk, so
//! editing a file that another one imports updates both.

use crate::text::Lines;
use cranium::ast::{Function, Item, Program, Type};
use cranium::lexer::Span;
use cranium::module::{self, Disk, Loader, SourceMap};
use std::collections::HashMap;
use std::path::Path;

/// What a name refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// A function.
    Function,
    /// A compile-time constant.
    Constant,
    /// A global variable.
    Variable,
}

impl SymbolKind {
    /// The protocol's numbering for this kind.
    pub fn lsp_code(self) -> i64 {
        match self {
            SymbolKind::Function => 12,
            SymbolKind::Constant => 14,
            SymbolKind::Variable => 13,
        }
    }
}

/// A top-level definition.
#[derive(Debug, Clone)]
pub struct Symbol {
    /// The name it is known by.
    pub name: String,
    /// What sort of thing it is.
    pub kind: SymbolKind,
    /// A one-line summary, as it would be written in source.
    pub detail: String,
    /// Where it is defined.
    pub span: Span,
}

/// The builtins the compiler provides, with the signature to show for each.
pub const BUILTINS: &[(&str, &str)] = &[
    (
        "print",
        "print(value, ...)  numbers in decimal, strings as text",
    ),
    ("println", "println(value, ...)  the same, then a newline"),
    ("putc", "putc(byte)  write one raw byte"),
    ("getc", "getc() -> byte  read one byte; 0 at end of input"),
    ("puts", "puts(array)  write a byte array up to its first 0"),
    ("len", "len(array) -> int  the array's length"),
];

/// Words that are part of the language rather than a program.
pub const KEYWORDS: &[&str] = &[
    "fn", "let", "const", "import", "if", "else", "while", "for", "in", "loop", "break",
    "continue", "return", "as", "true", "false",
];

/// The built-in types.
pub const TYPES: &[&str] = &["byte", "int", "sbyte", "sint", "bool"];

/// Open buffers, keyed the way the compiler names files.
#[derive(Debug, Default)]
pub struct Workspace {
    open: HashMap<String, Lines>,
}

/// The result of looking at a project once.
pub struct Analysis {
    /// The whole program, when it parsed.
    pub program: Option<Program>,
    /// The files it was built from.
    pub sources: SourceMap,
    /// The first problem found, if any.
    pub error: Option<cranium::Error>,
}

impl Workspace {
    /// The name the compiler will use for a path.
    pub fn key(path: &Path) -> String {
        Disk::entry_name(path)
    }

    /// Record or replace a buffer's contents.
    pub fn set(&mut self, path: &Path, text: String) {
        self.open.insert(Self::key(path), Lines::new(text));
    }

    /// Forget a buffer, so its file is read from disk again.
    pub fn remove(&mut self, path: &Path) {
        self.open.remove(&Self::key(path));
    }

    /// The buffer for a path, if it is open.
    pub fn document(&self, path: &Path) -> Option<&Lines> {
        self.open.get(&Self::key(path))
    }

    /// The text of a file the compiler named, from a buffer or from disk.
    pub fn lines_for(&self, name: &str) -> Option<Lines> {
        if let Some(open) = self.open.get(name) {
            return Some(open.clone());
        }
        std::fs::read_to_string(name).ok().map(Lines::new)
    }

    /// Compile a project, keeping both the tree and the first error.
    pub fn analyze(&self, entry: &Path) -> Analysis {
        let name = Self::key(entry);
        let mut loader = Overlay { open: &self.open };

        match module::gather(&name, &mut loader) {
            Ok(loaded) => {
                // Parsing succeeded, so go on to the part that finds undefined
                // names and type mismatches: lowering it for real.
                let error = cranium::codegen::compile(&loaded.program)
                    .err()
                    .map(|err| error_at(&loaded.sources, err.span, err.message));
                Analysis {
                    program: Some(loaded.program),
                    sources: loaded.sources,
                    error,
                }
            }
            Err(failure) => {
                let error = Some(gather_error(&failure.sources, &name, failure.error));
                Analysis {
                    program: None,
                    sources: failure.sources,
                    error,
                }
            }
        }
    }
}

fn error_at(sources: &SourceMap, span: Span, message: String) -> cranium::Error {
    cranium::Error {
        file: sources.name(span.file).to_string(),
        position: Some((span.line, span.column)),
        message,
    }
}

fn gather_error(sources: &SourceMap, entry: &str, error: module::GatherError) -> cranium::Error {
    match error {
        module::GatherError::Lex(err) => error_at(sources, err.span, err.message),
        module::GatherError::Parse(err) => error_at(sources, err.span, err.message),
        module::GatherError::Load(err) => match err.span {
            Some(span) => error_at(sources, span, err.message),
            None => cranium::Error {
                file: entry.to_string(),
                position: None,
                message: err.message,
            },
        },
    }
}

/// Serves open buffers first, falling back to the filesystem.
struct Overlay<'a> {
    open: &'a HashMap<String, Lines>,
}

impl Loader for Overlay<'_> {
    fn resolve(&mut self, from: &str, path: &str) -> Result<String, String> {
        Disk.resolve(from, path)
    }

    fn read(&mut self, name: &str) -> Result<String, String> {
        match self.open.get(name) {
            Some(open) => Ok(open.text().to_string()),
            None => Disk.read(name),
        }
    }
}

/// Every top-level definition in a program.
pub fn symbols(program: &Program) -> Vec<Symbol> {
    program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) => Some(Symbol {
                name: function.name.clone(),
                kind: SymbolKind::Function,
                detail: signature(function),
                span: function.span,
            }),
            Item::Const { name, ty, span, .. } => Some(Symbol {
                name: name.clone(),
                kind: SymbolKind::Constant,
                detail: match ty {
                    Some(ty) => format!("const {name}: {ty}"),
                    None => format!("const {name}"),
                },
                span: *span,
            }),
            Item::Global { name, ty, span, .. } => Some(Symbol {
                name: name.clone(),
                kind: SymbolKind::Variable,
                detail: match ty {
                    Some(ty) => format!("let {name}: {ty}"),
                    None => format!("let {name}"),
                },
                span: *span,
            }),
            // Imports are spliced away before this point.
            Item::Import { .. } => None,
        })
        .collect()
}

/// A function's signature, written the way the source would.
pub fn signature(function: &Function) -> String {
    let params = function
        .params
        .iter()
        .map(|param| format!("{}: {}", param.name, param.ty))
        .collect::<Vec<_>>()
        .join(", ");
    match &function.ret {
        Type::Unit => format!("fn {}({params})", function.name),
        ret => format!("fn {}({params}) -> {ret}", function.name),
    }
}

/// What to show when hovering over `word`.
pub fn describe(word: &str, program: Option<&Program>) -> Option<String> {
    if let Some(program) = program
        && let Some(symbol) = symbols(program).into_iter().find(|s| s.name == word)
    {
        return Some(symbol.detail);
    }
    if let Some((_, help)) = BUILTINS.iter().find(|(name, _)| *name == word) {
        return Some((*help).to_string());
    }
    if TYPES.contains(&word) {
        return Some(match word {
            "byte" => "byte  unsigned 8-bit, one cell, wrapping".to_string(),
            "int" => "int  unsigned 16-bit, two cells, wrapping".to_string(),
            "sbyte" => "sbyte  signed 8-bit, one cell, wrapping".to_string(),
            "sint" => "sint  signed 16-bit, two cells, wrapping".to_string(),
            _ => "bool  true or false, one cell".to_string(),
        });
    }
    if KEYWORDS.contains(&word) {
        return Some(format!("`{word}` is a keyword"));
    }
    None
}

/// Everything worth offering as a completion.
pub fn completions(program: Option<&Program>) -> Vec<(String, i64, String)> {
    // (label, LSP completion kind, detail)
    const KEYWORD: i64 = 14;
    const FUNCTION: i64 = 3;
    const CONSTANT: i64 = 21;
    const VARIABLE: i64 = 6;
    const TYPE: i64 = 22;

    let mut out: Vec<(String, i64, String)> = KEYWORDS
        .iter()
        .map(|word| ((*word).to_string(), KEYWORD, "keyword".to_string()))
        .chain(
            TYPES
                .iter()
                .map(|word| ((*word).to_string(), TYPE, "type".to_string())),
        )
        .chain(
            BUILTINS
                .iter()
                .map(|(name, help)| ((*name).to_string(), FUNCTION, (*help).to_string())),
        )
        .collect();

    if let Some(program) = program {
        for symbol in symbols(program) {
            let kind = match symbol.kind {
                SymbolKind::Function => FUNCTION,
                SymbolKind::Constant => CONSTANT,
                SymbolKind::Variable => VARIABLE,
            };
            out.push((symbol.name, kind, symbol.detail));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program_of(source: &str) -> Program {
        let tokens = cranium::lexer::tokenize(source, 0).expect("valid tokens");
        cranium::parser::parse(tokens).expect("valid program")
    }

    #[test]
    fn lists_top_level_definitions() {
        let program = program_of(
            "const K = 1;\nlet total: int = 0;\nfn add(a: byte, b: byte) -> int { return 0; }\nfn main() { }\n",
        );
        let found = symbols(&program);
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["K", "total", "add", "main"]);

        assert_eq!(found[0].kind, SymbolKind::Constant);
        assert_eq!(found[1].detail, "let total: int");
        assert_eq!(found[2].detail, "fn add(a: byte, b: byte) -> int");
        assert_eq!(found[3].detail, "fn main()");
    }

    #[test]
    fn describes_names_types_and_builtins() {
        let program = program_of("fn square(n: byte) -> int { return 0; }\n");
        assert_eq!(
            describe("square", Some(&program)).as_deref(),
            Some("fn square(n: byte) -> int")
        );
        assert!(describe("putc", None).unwrap().contains("raw byte"));
        assert!(describe("sint", None).unwrap().contains("signed 16-bit"));
        assert!(describe("while", None).unwrap().contains("keyword"));
        assert_eq!(describe("nothing_at_all", Some(&program)), None);
    }

    #[test]
    fn offers_keywords_builtins_and_program_names() {
        let program = program_of("const LIMIT = 5;\nfn helper() { }\n");
        let offered = completions(Some(&program));
        let labels: Vec<&str> = offered.iter().map(|(label, _, _)| label.as_str()).collect();

        assert!(labels.contains(&"while"));
        assert!(labels.contains(&"sbyte"));
        assert!(labels.contains(&"println"));
        assert!(labels.contains(&"LIMIT"));
        assert!(labels.contains(&"helper"));
    }
}
