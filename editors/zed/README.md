# Cranium for Zed

Syntax highlighting, an outline, and the [`wernicke`](../../crates/wernicke)
language server — diagnostics as you type, hover, go to definition, document
symbols, and completion.

## Installing

Install the language server first, since the extension only launches it:

```bash
cargo install --git https://github.com/Aspenini/bf-tools wernicke
```

Then in Zed run **`zed: install dev extension`** and pick this directory
(`editors/zed`). Open a `.cra` file and it should light up.

If `wernicke` is not on your `PATH`, point at it in your Zed settings:

```json
{
  "lsp": {
    "wernicke": {
      "binary": {
        "path": "/absolute/path/to/wernicke"
      }
    }
  }
}
```

## What is in here

| | |
| --- | --- |
| `extension.toml` | the manifest: which grammar, which language server |
| `src/lib.rs` | finds `wernicke` and hands Zed the command to run |
| `languages/cranium/config.toml` | file suffix, comments, brackets, indentation |
| `languages/cranium/*.scm` | highlighting, brackets, outline, indentation queries |

The grammar itself is [`editors/tree-sitter-cranium`](../tree-sitter-cranium),
in this repository rather than one of its own. `extension.toml` points at it
with a `path`, pinned to the commit that added it — so **bump `rev` whenever
the grammar changes**, or Zed will keep building the old one.

This directory is excluded from the Cargo workspace: Zed compiles it to
WebAssembly itself, and it is a `cdylib` rather than an ordinary crate.
