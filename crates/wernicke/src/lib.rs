//! Wernicke: a language server for [Cranium](https://github.com/Aspenini/bf-tools).
//!
//! The protocol is JSON-RPC over stdio, so this has no dependencies beyond the
//! compiler itself: [`json`] and [`rpc`] are a few hundred lines between them.
//!
//! There is no second model of the language here. Diagnostics come from
//! compiling the project for real, and hover, go-to-definition, symbols and
//! completion are read off the same syntax tree the compiler uses. Unsaved
//! buffers reach it through a [`cranium::Loader`], so editing a file another
//! one imports updates both.
//!
//! ```
//! use wernicke::{json::Json, server::Server};
//!
//! let mut server = Server::new();
//! let hello = Json::object([
//!     ("jsonrpc", Json::string("2.0")),
//!     ("id", Json::int(1)),
//!     ("method", Json::string("initialize")),
//! ]);
//! let replies = server.handle(&hello);
//! assert!(replies[0].path(&["result", "capabilities"]).is_some());
//! ```

#![warn(missing_docs)]

pub mod analysis;
pub mod json;
pub mod rpc;
pub mod server;
pub mod text;
