//! End-to-end tests: drive the server the way an editor would and read what
//! comes back.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use wernicke::json::{self, Json};
use wernicke::rpc;
use wernicke::server::Server;
use wernicke::text::path_to_uri;

/// A path that does not exist on disk, for testing an unsaved buffer.
fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("wernicke-test-{name}"))
}

fn uri_of(path: &Path) -> String {
    path_to_uri(path)
}

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

fn did_open(path: &Path, text: &str) -> Json {
    notify(
        "textDocument/didOpen",
        Json::object([(
            "textDocument",
            Json::object([
                ("uri", Json::string(uri_of(path))),
                ("languageId", Json::string("cranium")),
                ("version", Json::int(1)),
                ("text", Json::string(text)),
            ]),
        )]),
    )
}

fn did_change(path: &Path, text: &str) -> Json {
    notify(
        "textDocument/didChange",
        Json::object([
            (
                "textDocument",
                Json::object([
                    ("uri", Json::string(uri_of(path))),
                    ("version", Json::int(2)),
                ]),
            ),
            (
                "contentChanges",
                Json::array([Json::object([("text", Json::string(text))])]),
            ),
        ]),
    )
}

fn at(path: &Path, line: i64, character: i64) -> Json {
    Json::object([
        (
            "textDocument",
            Json::object([("uri", Json::string(uri_of(path)))]),
        ),
        (
            "position",
            Json::object([
                ("line", Json::int(line)),
                ("character", Json::int(character)),
            ]),
        ),
    ])
}

/// The diagnostics a batch of replies reports, as (uri, messages).
fn diagnostics(replies: &[Json]) -> Vec<(String, Vec<String>)> {
    replies
        .iter()
        .filter(|reply| {
            reply.get("method").and_then(Json::as_str) == Some("textDocument/publishDiagnostics")
        })
        .filter_map(|reply| {
            let params = reply.get("params")?;
            let uri = params.get("uri")?.as_str()?.to_string();
            let messages = params
                .get("diagnostics")?
                .as_array()?
                .iter()
                .filter_map(|item| Some(item.get("message")?.as_str()?.to_string()))
                .collect();
            Some((uri, messages))
        })
        .collect()
}

#[test]
fn reports_a_problem_and_then_takes_it_back() {
    let mut server = Server::new();
    let path = scratch("diagnostics.cra");

    let broken = server.handle(&did_open(&path, "fn main() {\n    let x = nope;\n}\n"));
    let reported = diagnostics(&broken);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert_eq!(reported[0].0, uri_of(&path));
    assert!(reported[0].1[0].contains("not defined"), "{reported:?}");

    let fixed = server.handle(&did_change(&path, "fn main() {\n    let x = 1;\n}\n"));
    let cleared = diagnostics(&fixed);
    assert_eq!(cleared.len(), 1, "{cleared:?}");
    assert!(cleared[0].1.is_empty(), "the problem should be withdrawn");
}

#[test]
fn a_diagnostic_underlines_the_word_rather_than_a_point() {
    let mut server = Server::new();
    let path = scratch("range.cra");
    let replies = server.handle(&did_open(&path, "fn main() {\n    let x = nope;\n}\n"));

    let params = replies
        .iter()
        .find(|reply| {
            reply.get("method").and_then(Json::as_str) == Some("textDocument/publishDiagnostics")
        })
        .and_then(|reply| reply.get("params"))
        .expect("a diagnostic");
    let range = params
        .path(&["diagnostics"])
        .and_then(Json::as_array)
        .and_then(<[Json]>::first)
        .and_then(|item| item.get("range"))
        .expect("a range");

    // `nope` sits on the second line, and the squiggle covers all four letters.
    assert_eq!(
        range.path(&["start", "line"]).and_then(Json::as_i64),
        Some(1)
    );
    let start = range
        .path(&["start", "character"])
        .and_then(Json::as_i64)
        .unwrap();
    let end = range
        .path(&["end", "character"])
        .and_then(Json::as_i64)
        .unwrap();
    assert_eq!(end - start, 4, "should cover `nope`");
}

#[test]
fn describes_what_the_cursor_is_on() {
    let mut server = Server::new();
    let path = scratch("hover.cra");
    let source = "fn square(n: byte) -> int {\n    return n as int * n as int;\n}\nfn main() { print(square(3)); }\n";
    server.handle(&did_open(&path, source));

    // The cursor sits inside `square` in the call on the last line.
    let replies = server.handle(&request(1, "textDocument/hover", at(&path, 3, 20)));
    let value = replies[0]
        .path(&["result", "contents", "value"])
        .and_then(Json::as_str)
        .expect("hover text");
    assert!(value.contains("fn square(n: byte) -> int"), "{value}");

    // A builtin is described too.
    let replies = server.handle(&request(2, "textDocument/hover", at(&path, 3, 13)));
    let value = replies[0]
        .path(&["result", "contents", "value"])
        .and_then(Json::as_str)
        .expect("hover text");
    assert!(value.contains("print"), "{value}");

    // Whitespace has nothing to say.
    let replies = server.handle(&request(3, "textDocument/hover", at(&path, 2, 0)));
    assert_eq!(replies[0].get("result"), Some(&Json::Null));
}

#[test]
fn jumps_from_a_call_to_its_definition() {
    let mut server = Server::new();
    let path = scratch("definition.cra");
    let source = "fn helper() -> byte {\n    return 1;\n}\nfn main() { print(helper()); }\n";
    server.handle(&did_open(&path, source));

    let replies = server.handle(&request(1, "textDocument/definition", at(&path, 3, 20)));
    let result = replies[0].get("result").expect("a location");

    assert_eq!(
        result.get("uri").and_then(Json::as_str),
        Some(uri_of(&path)).as_deref()
    );
    // `helper` is defined on the first line.
    assert_eq!(
        result
            .path(&["range", "start", "line"])
            .and_then(Json::as_i64),
        Some(0)
    );
}

#[test]
fn lists_the_definitions_in_a_file() {
    let mut server = Server::new();
    let path = scratch("symbols.cra");
    let source = "const K = 1;\nlet total: int = 0;\nfn helper() { }\nfn main() { }\n";
    server.handle(&did_open(&path, source));

    let replies = server.handle(&request(1, "textDocument/documentSymbol", at(&path, 0, 0)));
    let symbols = replies[0]
        .get("result")
        .and_then(Json::as_array)
        .expect("symbols");

    let names: Vec<&str> = symbols
        .iter()
        .filter_map(|s| s.get("name").and_then(Json::as_str))
        .collect();
    assert_eq!(names, ["K", "total", "helper", "main"]);

    let details: Vec<&str> = symbols
        .iter()
        .filter_map(|s| s.get("detail").and_then(Json::as_str))
        .collect();
    assert!(details.contains(&"let total: int"), "{details:?}");
}

#[test]
fn offers_completions_from_the_program_and_the_language() {
    let mut server = Server::new();
    let path = scratch("completion.cra");
    server.handle(&did_open(
        &path,
        "const LIMIT = 5;\nfn helper() { }\nfn main() { }\n",
    ));

    let replies = server.handle(&request(1, "textDocument/completion", at(&path, 2, 12)));
    let items = replies[0]
        .get("result")
        .and_then(Json::as_array)
        .expect("completions");
    let labels: Vec<&str> = items
        .iter()
        .filter_map(|item| item.get("label").and_then(Json::as_str))
        .collect();

    for expected in ["while", "sint", "println", "LIMIT", "helper"] {
        assert!(labels.contains(&expected), "missing {expected}: {labels:?}");
    }
}

/// Editing a file that another one imports must report the problem against the
/// file it is in, using the unsaved buffer rather than what is on disk.
#[test]
fn follows_imports_into_unsaved_buffers() {
    // Editors always send absolute URIs, so resolve the example before use.
    let project = Path::new("../cranium/examples/project")
        .canonicalize()
        .expect("the multi-file example should be in the tree");
    let main = project.join("main.cra");
    let library = project.join("lib").join("text.cra");

    let mut server = Server::new();
    let on_disk = std::fs::read_to_string(&library).expect("readable");

    // Opening the project as it stands reports nothing.
    let clean = server.handle(&did_open(&main, &std::fs::read_to_string(&main).unwrap()));
    for (uri, messages) in diagnostics(&clean) {
        assert!(
            messages.is_empty(),
            "unexpected problem in {uri}: {messages:?}"
        );
    }

    // Break the imported file in the editor only, and the error is attributed
    // to that file even though the entry point is what gets compiled.
    server.handle(&did_open(&library, &on_disk));
    let broken = server.handle(&did_change(
        &library,
        &format!("{on_disk}\nfn broken() {{ let x = definitely_not_defined; }}\n"),
    ));

    // The mistake is inside a function nothing calls, which Cranium does not
    // compile, so add a call from the entry point to make it reachable.
    let main_source = std::fs::read_to_string(&main).unwrap();
    let replies = server.handle(&did_change(
        &main,
        &main_source.replace("fn main() {", "fn main() {\n    broken();"),
    ));

    let reported = diagnostics(&replies);
    let complaint = reported
        .iter()
        .find(|(_, messages)| !messages.is_empty())
        .unwrap_or_else(|| panic!("expected a problem, got {reported:?}; earlier {broken:?}"));
    assert!(
        complaint.0.contains("text.cra"),
        "should blame the imported file, got {}",
        complaint.0
    );
    assert!(complaint.1[0].contains("not defined"), "{:?}", complaint.1);
}

#[test]
fn runs_a_whole_session_over_the_pipe() {
    let path = scratch("session.cra");
    let mut wire = Vec::new();
    for message in [
        request(1, "initialize", Json::Null),
        notify("initialized", Json::Null),
        did_open(&path, "fn main() { print(1); }\n"),
        request(2, "textDocument/documentSymbol", at(&path, 0, 0)),
        request(3, "shutdown", Json::Null),
        notify("exit", Json::Null),
    ] {
        rpc::write_message(&mut wire, &message).expect("writes");
    }

    let mut input = Cursor::new(wire);
    let mut output = Vec::new();
    Server::new()
        .serve(&mut input, &mut output)
        .expect("the session runs");

    // Read the replies back off the wire the way an editor would.
    let mut replies = Vec::new();
    let mut reader = Cursor::new(output);
    while let Some(message) = rpc::read_message(&mut reader).expect("reads") {
        replies.push(message);
    }

    let ids: Vec<i64> = replies
        .iter()
        .filter_map(|reply| reply.get("id").and_then(Json::as_i64))
        .collect();
    assert_eq!(ids, [1, 2, 3], "every request should be answered once");
    assert!(replies[0].path(&["result", "capabilities"]).is_some());
}

#[test]
fn survives_nonsense_without_falling_over() {
    let mut server = Server::new();

    // A request for something unsupported still gets an answer.
    let replies = server.handle(&request(9, "textDocument/rename", Json::Null));
    assert_eq!(replies.len(), 1);
    assert!(replies[0].get("error").is_some());

    // Notifications with missing or malformed parameters are ignored.
    assert!(server
        .handle(&notify("textDocument/didChange", Json::Null))
        .is_empty());
    assert!(server
        .handle(&notify(
            "textDocument/didOpen",
            Json::object([("textDocument", Json::Null)])
        ))
        .is_empty());

    // A message that is not a request or a known notification is ignored.
    let junk = json::parse(r#"{"jsonrpc":"2.0"}"#).expect("valid json");
    assert!(server.handle(&junk).is_empty());
}
