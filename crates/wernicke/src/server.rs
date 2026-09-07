//! Request dispatch: what the editor asks for, and what it gets back.

use crate::analysis::{self, Workspace};
use crate::json::Json;
use crate::rpc;
use crate::text::{self, Lines};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// Error codes from the protocol that this server uses.
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_REQUEST: i64 = -32600;

/// Diagnostic severity: an error.
const SEVERITY_ERROR: i64 = 1;

/// The language server.
pub struct Server {
    workspace: Workspace,
    /// Set once the editor asks to shut down, so `exit` can report cleanly.
    shutting_down: bool,
    /// Files a diagnostic was last published for, so they can be cleared.
    published: Vec<String>,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    /// A server with nothing open.
    pub fn new() -> Self {
        Self {
            workspace: Workspace::default(),
            shutting_down: false,
            published: Vec::new(),
        }
    }

    /// Serve until the input ends or the editor asks to exit.
    ///
    /// # Errors
    ///
    /// Fails when reading or writing the pipe fails.
    pub fn serve(
        &mut self,
        input: &mut impl BufRead,
        output: &mut impl Write,
    ) -> std::io::Result<()> {
        while let Some(message) = rpc::read_message(input)? {
            for reply in self.handle(&message) {
                rpc::write_message(output, &reply)?;
            }
            if message.get("method").and_then(Json::as_str) == Some("exit") {
                break;
            }
        }
        Ok(())
    }

    /// Handle one message, returning everything to send back.
    pub fn handle(&mut self, message: &Json) -> Vec<Json> {
        let method = message.get("method").and_then(Json::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Json::Null);

        match (method, id) {
            ("initialize", Some(id)) => vec![rpc::response(id, capabilities())],
            ("shutdown", Some(id)) => {
                self.shutting_down = true;
                vec![rpc::response(id, Json::Null)]
            }
            ("initialized" | "exit", _) => Vec::new(),

            ("textDocument/didOpen", _) => {
                let Some((path, text)) = opened_document(&params) else {
                    return Vec::new();
                };
                self.workspace.set(&path, text);
                self.publish(&path)
            }
            ("textDocument/didChange", _) => {
                let Some(path) = document_path(&params) else {
                    return Vec::new();
                };
                // The server asks for whole-document sync, so the last change
                // holds the entire new text.
                let Some(text) = params
                    .get("contentChanges")
                    .and_then(Json::as_array)
                    .and_then(<[Json]>::last)
                    .and_then(|change| change.get("text"))
                    .and_then(Json::as_str)
                else {
                    return Vec::new();
                };
                self.workspace.set(&path, text.to_string());
                self.publish(&path)
            }
            ("textDocument/didSave", _) => match document_path(&params) {
                Some(path) => self.publish(&path),
                None => Vec::new(),
            },
            ("textDocument/didClose", _) => {
                if let Some(path) = document_path(&params) {
                    self.workspace.remove(&path);
                }
                Vec::new()
            }

            ("textDocument/documentSymbol", Some(id)) => {
                vec![rpc::response(id, self.document_symbols(&params))]
            }
            ("textDocument/hover", Some(id)) => vec![rpc::response(id, self.hover(&params))],
            ("textDocument/definition", Some(id)) => {
                vec![rpc::response(id, self.definition(&params))]
            }
            ("textDocument/completion", Some(id)) => {
                vec![rpc::response(id, self.completion(&params))]
            }

            // Every other request still needs an answer, or the editor waits.
            (_, Some(id)) => vec![rpc::error_response(
                id,
                if method.is_empty() {
                    INVALID_REQUEST
                } else {
                    METHOD_NOT_FOUND
                },
                &format!("unsupported method `{method}`"),
            )],
            (_, None) => Vec::new(),
        }
    }

    /// Compile the project this file belongs to and publish what came back.
    ///
    /// Diagnostics are published for every file that had one and cleared for
    /// every file that no longer does, including files reached by `import`.
    fn publish(&mut self, path: &Path) -> Vec<Json> {
        let analysis = self.workspace.analyze(path);
        let mut messages = Vec::new();
        let mut now_published = Vec::new();

        if let Some(error) = &analysis.error {
            let range = self.range_of(&error.file, error.position);
            let diagnostic = Json::object([
                ("range", range),
                ("severity", Json::int(SEVERITY_ERROR)),
                ("source", Json::string("cranium")),
                ("message", Json::string(error.message.clone())),
            ]);
            messages.push(diagnostics_for(&error.file, Json::array([diagnostic])));
            now_published.push(error.file.clone());
        }

        // Anything that was complaining and no longer is gets an empty list,
        // which is how the protocol says a problem went away.
        for stale in &self.published {
            if !now_published.contains(stale) {
                messages.push(diagnostics_for(stale, Json::array([])));
            }
        }
        // The file being edited always gets a report, even a clean one.
        let current = Workspace::key(path);
        if !now_published.contains(&current) && !self.published.contains(&current) {
            messages.push(diagnostics_for(&current, Json::array([])));
        }

        self.published = now_published;
        messages
    }

    /// The range to underline for a problem, widened onto the whole word.
    fn range_of(&self, file: &str, position: Option<(usize, usize)>) -> Json {
        let Some((line, column)) = position else {
            return range(0, 0, 0, 0);
        };
        let Some(lines) = self.workspace.lines_for(file) else {
            let start = (line.saturating_sub(1), column.saturating_sub(1));
            return range(start.0, start.1, start.0, start.1 + 1);
        };
        let (row, character) = lines.to_position(line, column);
        match lines.word_at(row, column.saturating_sub(1)) {
            Some((word, start_byte)) => {
                let start = lines.to_position(line, start_byte + 1).1;
                range(row, start, row, start + word.encode_utf16().count())
            }
            None => range(row, character, row, character + 1),
        }
    }

    fn document_symbols(&self, params: &Json) -> Json {
        let Some(path) = document_path(params) else {
            return Json::array([]);
        };
        let analysis = self.workspace.analyze(&path);
        let Some(program) = &analysis.program else {
            return Json::array([]);
        };
        let wanted = Workspace::key(&path);

        let symbols = analysis::symbols(program)
            .into_iter()
            .filter(|symbol| analysis.sources.name(symbol.span.file) == wanted)
            .filter_map(|symbol| {
                let lines = self.workspace.lines_for(&wanted)?;
                let (row, character) = lines.to_position(symbol.span.line, symbol.span.column);
                let width = symbol.name.encode_utf16().count();
                Some(Json::object([
                    ("name", Json::string(symbol.name.clone())),
                    ("detail", Json::string(symbol.detail)),
                    ("kind", Json::int(symbol.kind.lsp_code())),
                    ("range", range(row, character, row, character + width)),
                    (
                        "selectionRange",
                        range(row, character, row, character + width),
                    ),
                ]))
            });
        Json::array(symbols)
    }

    fn hover(&self, params: &Json) -> Json {
        let Some((path, word, _)) = self.word_under_cursor(params) else {
            return Json::Null;
        };
        let analysis = self.workspace.analyze(&path);
        let Some(text) = analysis::describe(&word, analysis.program.as_ref()) else {
            return Json::Null;
        };
        Json::object([(
            "contents",
            Json::object([
                ("kind", Json::string("markdown")),
                ("value", Json::string(format!("```cranium\n{text}\n```"))),
            ]),
        )])
    }

    fn definition(&self, params: &Json) -> Json {
        let Some((path, word, _)) = self.word_under_cursor(params) else {
            return Json::Null;
        };
        let analysis = self.workspace.analyze(&path);
        let Some(program) = &analysis.program else {
            return Json::Null;
        };
        let Some(symbol) = analysis::symbols(program)
            .into_iter()
            .find(|symbol| symbol.name == word)
        else {
            return Json::Null;
        };

        let file = analysis.sources.name(symbol.span.file).to_string();
        let Some(lines) = self.workspace.lines_for(&file) else {
            return Json::Null;
        };
        let (row, character) = lines.to_position(symbol.span.line, symbol.span.column);
        let width = symbol.name.encode_utf16().count();
        Json::object([
            ("uri", Json::string(text::path_to_uri(Path::new(&file)))),
            ("range", range(row, character, row, character + width)),
        ])
    }

    fn completion(&self, params: &Json) -> Json {
        let path = document_path(params);
        let analysis = path.as_ref().map(|path| self.workspace.analyze(path));
        let program = analysis.as_ref().and_then(|a| a.program.as_ref());

        let items = analysis::completions(program)
            .into_iter()
            .map(|(label, kind, detail)| {
                Json::object([
                    ("label", Json::string(label)),
                    ("kind", Json::int(kind)),
                    ("detail", Json::string(detail)),
                ])
            });
        Json::array(items)
    }

    /// The identifier the cursor is resting on, and where it starts.
    fn word_under_cursor(&self, params: &Json) -> Option<(PathBuf, String, usize)> {
        let path = document_path(params)?;
        let line = usize::try_from(params.path(&["position", "line"])?.as_i64()?).ok()?;
        let character = usize::try_from(params.path(&["position", "character"])?.as_i64()?).ok()?;
        let lines = self.workspace.document(&path)?;
        let column = lines.to_byte_column(line, character);
        let (word, start) = lines.word_at(line, column)?;
        Some((path, word, start))
    }
}

fn diagnostics_for(file: &str, list: Json) -> Json {
    rpc::notification(
        "textDocument/publishDiagnostics",
        Json::object([
            ("uri", Json::string(text::path_to_uri(Path::new(file)))),
            ("diagnostics", list),
        ]),
    )
}

fn range(start_line: usize, start_char: usize, end_line: usize, end_char: usize) -> Json {
    Json::object([
        (
            "start",
            Json::object([
                ("line", Json::int(start_line as i64)),
                ("character", Json::int(start_char as i64)),
            ]),
        ),
        (
            "end",
            Json::object([
                ("line", Json::int(end_line as i64)),
                ("character", Json::int(end_char as i64)),
            ]),
        ),
    ])
}

fn document_path(params: &Json) -> Option<PathBuf> {
    let uri = params.path(&["textDocument", "uri"])?.as_str()?;
    text::uri_to_path(uri)
}

fn opened_document(params: &Json) -> Option<(PathBuf, String)> {
    let document = params.get("textDocument")?;
    let path = text::uri_to_path(document.get("uri")?.as_str()?)?;
    let text = document.get("text")?.as_str()?.to_string();
    Some((path, text))
}

/// What this server can do, in the shape `initialize` expects.
fn capabilities() -> Json {
    Json::object([
        (
            "capabilities",
            Json::object([
                // 1 is full-document sync: every change carries the whole file,
                // which is simple and fast enough for source this size.
                ("textDocumentSync", Json::int(1)),
                ("hoverProvider", Json::Bool(true)),
                ("definitionProvider", Json::Bool(true)),
                ("documentSymbolProvider", Json::Bool(true)),
                (
                    "completionProvider",
                    Json::object([("triggerCharacters", Json::array([]))]),
                ),
            ]),
        ),
        (
            "serverInfo",
            Json::object([
                ("name", Json::string("wernicke")),
                ("version", Json::string(env!("CARGO_PKG_VERSION"))),
            ]),
        ),
    ])
}

/// A line index for text that is not attached to a file.
pub fn lines_of(text: &str) -> Lines {
    Lines::new(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    fn request(id: i64, method: &str, params: Json) -> Json {
        Json::object([
            ("jsonrpc", Json::string("2.0")),
            ("id", Json::int(id)),
            ("method", Json::string(method)),
            ("params", params),
        ])
    }

    fn notify(method: &str, params: Json) -> Json {
        Json::object([
            ("jsonrpc", Json::string("2.0")),
            ("method", Json::string(method)),
            ("params", params),
        ])
    }

    #[test]
    fn announces_what_it_supports() {
        let mut server = Server::new();
        let replies = server.handle(&request(1, "initialize", Json::Null));
        let caps = replies[0].path(&["result", "capabilities"]).expect("caps");

        assert_eq!(caps.get("textDocumentSync").and_then(Json::as_i64), Some(1));
        assert_eq!(caps.get("hoverProvider"), Some(&Json::Bool(true)));
        assert_eq!(caps.get("definitionProvider"), Some(&Json::Bool(true)));
        assert!(caps.get("completionProvider").is_some());
    }

    #[test]
    fn answers_an_unknown_request_rather_than_leaving_it_hanging() {
        let mut server = Server::new();
        let replies = server.handle(&request(7, "textDocument/formatting", Json::Null));

        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].get("id").and_then(Json::as_i64), Some(7));
        assert_eq!(
            replies[0].path(&["error", "code"]).and_then(Json::as_i64),
            Some(METHOD_NOT_FOUND)
        );
    }

    #[test]
    fn ignores_notifications_it_does_not_handle() {
        let mut server = Server::new();
        assert!(server.handle(&notify("initialized", Json::Null)).is_empty());
        assert!(server.handle(&notify("$/setTrace", Json::Null)).is_empty());
    }

    #[test]
    fn parses_a_real_message_off_the_wire() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"shutdown"}"#;
        let message = json::parse(body).expect("valid");
        let mut server = Server::new();
        let replies = server.handle(&message);

        assert_eq!(replies[0].get("result"), Some(&Json::Null));
        assert!(server.shutting_down);
    }
}
