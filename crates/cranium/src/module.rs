//! Turning a file and its `import`s into one program.
//!
//! Cranium has no runtime linker to defer anything to, so imports are resolved
//! before compiling: every file is read once, its items are spliced into a
//! single [`Program`], and the result is compiled exactly as a single file
//! would be. There is one namespace across the whole program, so two files
//! cannot both define `helper`.
//!
//! A file's imports are placed ahead of its own items, which is what makes
//! `const` and global initializers see the definitions they depend on.
//! Importing the same file twice is not an error - the second import is
//! ignored, so diamonds work - but a cycle is, because there is no order in
//! which such files could be laid out.

use crate::ast::{Item, Program};
use crate::lexer::{self, FileId, Span};
use crate::parser;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

/// The files a program was built from, so a [`Span`] can name one.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    names: Vec<String>,
}

impl SourceMap {
    /// Record a file and return the id spans in it will carry.
    fn add(&mut self, name: &str) -> FileId {
        self.names.push(name.to_string());
        (self.names.len() - 1) as FileId
    }

    /// The name a file was read under.
    pub fn name(&self, file: FileId) -> &str {
        self.names
            .get(file as usize)
            .map(String::as_str)
            .unwrap_or("<unknown>")
    }

    /// How many files the program was built from.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// True when no file has been recorded.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Every file name, in the order they were read.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }
}

/// A problem finding or reading an imported file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    /// Human readable description.
    pub message: String,
    /// The import that caused it, when there was one.
    pub span: Option<Span>,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// The library that ships inside the compiler.
///
/// `import "std/gfx.cra"` is answered from here rather than from disk, so a
/// program can use it with nothing installed beside it and no copy to keep up
/// to date. Every [`Loader`] gets this, because it is handled before the
/// loader is asked.
pub const STD: &[(&str, &str)] = &[("std/gfx.cra", include_str!("../std/gfx.cra"))];

/// The prefix that names the bundled library.
///
/// It is reserved: a directory called `std` beside a program does not shadow
/// it, so an import means the same thing wherever the program is compiled.
pub const STD_PREFIX: &str = "std/";

/// Return the bundled source for `name`, if there is one.
pub fn std_source(name: &str) -> Option<&'static str> {
    STD.iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, source)| *source)
}

/// Resolve an import written inside `from` against the bundled library.
///
/// Returns `None` when the import is nothing to do with the library, so the
/// caller should ask the loader instead.
fn resolve_std(from: &str, path: &str) -> Option<Result<String, String>> {
    // A file in the library may import a sibling by its bare name.
    let name = if path.starts_with(STD_PREFIX) {
        path.to_string()
    } else if from.starts_with(STD_PREFIX) {
        format!("{STD_PREFIX}{path}")
    } else {
        return None;
    };

    if std_source(&name).is_some() {
        return Some(Ok(name));
    }

    Some(Err(format!(
        "no `{name}` in the standard library; it has {}",
        STD.iter()
            .map(|(candidate, _)| *candidate)
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// Where source text comes from.
///
/// Splitting this out keeps the compiler testable without a filesystem, and
/// leaves room for reading a project out of somewhere other than disk.
pub trait Loader {
    /// Turn `path`, as written in an import inside `from`, into the canonical
    /// name of the file it means. The same file must always produce the same
    /// name, since that is what stops it being read twice.
    fn resolve(&mut self, from: &str, path: &str) -> Result<String, String>;

    /// Read a file by canonical name.
    fn read(&mut self, name: &str) -> Result<String, String>;
}

/// Reads files from the filesystem, resolving imports relative to the file
/// they appear in.
#[derive(Debug, Clone, Copy, Default)]
pub struct Disk;

impl Disk {
    /// The canonical name for an entry point, so that importing it back by a
    /// different spelling still counts as the same file.
    pub fn entry_name(path: &Path) -> String {
        canonical(path)
    }
}

impl Loader for Disk {
    fn resolve(&mut self, from: &str, path: &str) -> Result<String, String> {
        let base = Path::new(from).parent().unwrap_or_else(|| Path::new("."));
        let joined = base.join(path);
        if !joined.exists() {
            return Err(format!("cannot find `{}`", joined.display()));
        }
        Ok(canonical(&joined))
    }

    fn read(&mut self, name: &str) -> Result<String, String> {
        std::fs::read_to_string(name).map_err(|err| format!("cannot read `{name}`: {err}"))
    }
}

/// Canonicalize where possible, and otherwise keep the path as written, so a
/// missing file still gets a name worth printing.
fn canonical(path: &Path) -> String {
    match path.canonicalize() {
        // Windows canonicalization prefixes a verbatim marker that is only
        // noise in a diagnostic.
        Ok(full) => {
            let text = full.display().to_string();
            text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
        }
        Err(_) => path.display().to_string(),
    }
}

/// Serves files held in memory, keyed by exactly the name an import writes.
#[derive(Debug, Clone, Default)]
pub struct Memory {
    files: HashMap<String, String>,
}

impl Memory {
    /// Build a loader over `(name, source)` pairs.
    pub fn new<N, S>(files: impl IntoIterator<Item = (N, S)>) -> Self
    where
        N: Into<String>,
        S: Into<String>,
    {
        Self {
            files: files
                .into_iter()
                .map(|(name, source)| (name.into(), source.into()))
                .collect(),
        }
    }
}

impl Loader for Memory {
    fn resolve(&mut self, _from: &str, path: &str) -> Result<String, String> {
        if self.files.contains_key(path) {
            Ok(path.to_string())
        } else {
            Err(format!("cannot find `{path}`"))
        }
    }

    fn read(&mut self, name: &str) -> Result<String, String> {
        self.files
            .get(name)
            .cloned()
            .ok_or_else(|| format!("cannot read `{name}`"))
    }
}

/// Refuses every import, for callers compiling a single detached string.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoImports;

impl Loader for NoImports {
    fn resolve(&mut self, _from: &str, path: &str) -> Result<String, String> {
        Err(format!(
            "cannot import `{path}`: this source has no file to resolve it against"
        ))
    }

    fn read(&mut self, name: &str) -> Result<String, String> {
        Err(format!("cannot read `{name}`"))
    }
}

/// A program gathered from one or more files.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// Every item, imports first and already spliced in.
    pub program: Program,
    /// The files it came from.
    pub sources: SourceMap,
}

/// What went wrong while gathering a program.
#[derive(Debug, Clone, PartialEq)]
pub enum GatherError {
    /// A file could not be found or read.
    Load(LoadError),
    /// A file could not be tokenized.
    Lex(lexer::LexError),
    /// A file could not be parsed.
    Parse(parser::ParseError),
}

impl fmt::Display for GatherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GatherError::Load(err) => write!(f, "{err}"),
            GatherError::Lex(err) => write!(f, "{err}"),
            GatherError::Parse(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for GatherError {}

struct Gatherer<'a> {
    loader: &'a mut dyn Loader,
    sources: SourceMap,
    items: Vec<Item>,
    finished: HashSet<String>,
    in_progress: Vec<String>,
}

/// A failed gather, carrying the files read so far so the error can still name
/// the one it happened in.
#[derive(Debug, Clone)]
pub struct GatherFailure {
    /// Files read before the failure.
    pub sources: SourceMap,
    /// What went wrong.
    pub error: GatherError,
}

/// Read `entry` and everything it imports into a single program.
///
/// # Errors
///
/// Returns the first file that could not be found, read, tokenized, or parsed,
/// or a [`GatherError::Load`] describing an import cycle.
pub fn gather(entry: &str, loader: &mut dyn Loader) -> Result<Loaded, GatherFailure> {
    let mut gatherer = Gatherer {
        loader,
        sources: SourceMap::default(),
        items: Vec::new(),
        finished: HashSet::new(),
        in_progress: Vec::new(),
    };
    match gatherer.visit(entry, None) {
        Ok(()) => Ok(Loaded {
            program: Program {
                items: gatherer.items,
            },
            sources: gatherer.sources,
        }),
        Err(error) => Err(GatherFailure {
            sources: gatherer.sources,
            error,
        }),
    }
}

impl Gatherer<'_> {
    fn visit(&mut self, name: &str, from: Option<Span>) -> Result<(), GatherError> {
        if self.finished.contains(name) {
            return Ok(());
        }
        if let Some(start) = self.in_progress.iter().position(|seen| seen == name) {
            let mut chain: Vec<&str> = self.in_progress[start..]
                .iter()
                .map(String::as_str)
                .collect();
            chain.push(name);
            return Err(GatherError::Load(LoadError {
                message: format!("import cycle: {}", chain.join(" -> ")),
                span: from,
            }));
        }

        let source = match std_source(name) {
            Some(source) => source.to_string(),
            None => self.loader.read(name).map_err(|message| {
                GatherError::Load(LoadError {
                    message,
                    span: from,
                })
            })?,
        };

        let file = self.sources.add(name);
        let tokens = lexer::tokenize(&source, file).map_err(GatherError::Lex)?;
        let program = parser::parse(tokens).map_err(GatherError::Parse)?;

        self.in_progress.push(name.to_string());
        let mut own = Vec::new();
        for item in program.items {
            match item {
                Item::Import { path, span } => {
                    // The bundled library is answered before the loader is
                    // asked, so it reaches every loader and cannot be shadowed
                    // by a directory that happens to be called `std`.
                    let resolved = resolve_std(name, &path)
                        .unwrap_or_else(|| self.loader.resolve(name, &path));
                    let target = resolved.map_err(|message| {
                        GatherError::Load(LoadError {
                            message,
                            span: Some(span),
                        })
                    })?;
                    self.visit(&target, Some(span))?;
                }
                other => own.push(other),
            }
        }
        self.in_progress.pop();
        self.finished.insert(name.to_string());

        // A file's own items come after everything it imported, so constants
        // and globals it depends on are already in place.
        self.items.extend(own);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names_of(loaded: &Loaded) -> Vec<&str> {
        loaded.sources.names().collect()
    }

    #[test]
    fn the_bundled_library_needs_no_filesystem() {
        // `Memory` knows nothing about `std/gfx.cra`, and does not have to.
        let mut loader = Memory::new([(
            "main.cra",
            "import \"std/gfx.cra\";
fn shade(x: byte, y: byte) { set_rgb(x, y, 0); }
fn main() { }
",
        )]);
        let loaded = gather("main.cra", &mut loader).expect("loads");

        assert_eq!(names_of(&loaded), ["main.cra", "std/gfx.cra"]);
    }

    #[test]
    fn a_local_std_directory_does_not_shadow_the_bundled_one() {
        // The prefix is reserved, so an import means the same thing wherever
        // the program is compiled.
        let mut loader = Memory::new([
            (
                "main.cra",
                "import \"std/gfx.cra\";
fn main() { }
",
            ),
            (
                "std/gfx.cra",
                "const GFX_IMPOSTOR = 1;
",
            ),
        ]);
        let loaded = gather("main.cra", &mut loader).expect("loads");

        assert!(
            !loaded
                .program
                .items
                .iter()
                .any(|item| matches!(item, Item::Const { name, .. } if name == "GFX_IMPOSTOR")),
            "a local file shadowed the bundled library"
        );
    }

    #[test]
    fn an_unknown_bundled_file_says_what_there_is() {
        let mut loader = Memory::new([(
            "main.cra",
            "import \"std/nope.cra\";
fn main() { }
",
        )]);
        let failure = gather("main.cra", &mut loader).expect_err("no such file");

        let GatherError::Load(error) = failure.error else {
            panic!("expected a load error");
        };
        assert!(error.message.contains("std/nope.cra"), "{}", error.message);
        assert!(error.message.contains("std/gfx.cra"), "{}", error.message);
    }

    #[test]
    fn a_bundled_file_can_import_a_sibling_by_bare_name() {
        assert_eq!(
            resolve_std("std/gfx.cra", "gfx.cra"),
            Some(Ok("std/gfx.cra".to_string()))
        );
        // An ordinary file's ordinary import is left to the loader.
        assert_eq!(resolve_std("main.cra", "lib.cra"), None);
    }

    #[test]
    fn every_bundled_file_parses() {
        for (name, source) in STD {
            let mut sources = SourceMap::default();
            let file = sources.add(name);
            let tokens = lexer::tokenize(source, file)
                .unwrap_or_else(|error| panic!("{name}: {}", error.message));
            parser::parse(tokens).unwrap_or_else(|error| panic!("{name}: {}", error.message));
        }
    }

    #[test]
    fn splices_imported_items_ahead_of_the_importer() {
        let mut loader = Memory::new([
            ("main.cra", "import \"lib.cra\";\nfn main() { }\n"),
            ("lib.cra", "const K = 7;\n"),
        ]);
        let loaded = gather("main.cra", &mut loader).expect("loads");

        assert_eq!(names_of(&loaded), ["main.cra", "lib.cra"]);
        assert!(matches!(loaded.program.items[0], Item::Const { .. }));
        assert!(matches!(loaded.program.items[1], Item::Function(_)));
    }

    #[test]
    fn reads_a_shared_file_once() {
        let mut loader = Memory::new([
            (
                "main.cra",
                "import \"a.cra\";\nimport \"b.cra\";\nfn main() { }\n",
            ),
            ("a.cra", "import \"shared.cra\";\nconst A = 1;\n"),
            ("b.cra", "import \"shared.cra\";\nconst B = 2;\n"),
            ("shared.cra", "const S = 3;\n"),
        ]);
        let loaded = gather("main.cra", &mut loader).expect("loads");

        assert_eq!(loaded.sources.len(), 4);
        assert_eq!(loaded.program.items.len(), 4);
    }

    #[test]
    fn reports_an_import_cycle() {
        let mut loader = Memory::new([
            ("a.cra", "import \"b.cra\";\nfn main() { }\n"),
            ("b.cra", "import \"a.cra\";\n"),
        ]);
        let err = gather("a.cra", &mut loader).expect_err("cycle").error;

        let GatherError::Load(err) = err else {
            panic!("expected a load error, got {err:?}");
        };
        assert!(err.message.contains("import cycle"), "{}", err.message);
        assert!(err.message.contains("a.cra"), "{}", err.message);
    }

    #[test]
    fn reports_a_missing_import() {
        let mut loader = Memory::new([("main.cra", "import \"nope.cra\";\n")]);
        let err = gather("main.cra", &mut loader)
            .expect_err("missing file")
            .error;

        let GatherError::Load(err) = err else {
            panic!("expected a load error, got {err:?}");
        };
        assert!(err.message.contains("nope.cra"), "{}", err.message);
        assert!(err.span.is_some(), "the failing import should be located");
    }

    #[test]
    fn spans_name_the_file_they_came_from() {
        let mut loader = Memory::new([
            ("main.cra", "import \"lib.cra\";\nfn main() { }\n"),
            ("lib.cra", "const K = 7;\n"),
        ]);
        let loaded = gather("main.cra", &mut loader).expect("loads");

        let Item::Const { span, .. } = &loaded.program.items[0] else {
            panic!("expected the imported constant first");
        };
        assert_eq!(loaded.sources.name(span.file), "lib.cra");
    }

    #[test]
    fn refuses_imports_from_a_detached_source() {
        let mut loader = Memory::new([("<source>", "import \"lib.cra\";\n")]);
        let err = gather("<source>", &mut loader)
            .expect_err("no such file")
            .error;
        assert!(matches!(err, GatherError::Load(_)));
    }
}
