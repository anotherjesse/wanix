//! Thin wrapper over `brush-parser` that isolates the dependency.
//!
//! The rest of the crate never imports `brush_parser` types except through the
//! returned [`ast::Program`], so the syntax front-end can be swapped or upgraded
//! in one place.

use brush_parser::{ParserOptions, ast, parse_tokens, tokenize_str};

use crate::error::{ShellError, ShellResult};

/// Parses a shell program string into a `brush-parser` AST.
///
/// This performs tokenization and grammar parsing only. It does **not** expand,
/// glob, or evaluate anything — that is the executor's job.
///
/// # Errors
///
/// Returns [`ShellError::Parse`] if the input is not valid shell syntax.
pub fn parse_program(input: &str) -> ShellResult<ast::Program> {
    let tokens = tokenize_str(input).map_err(|err| ShellError::Parse(err.to_string()))?;
    parse_tokens(&tokens, &ParserOptions::default())
        .map_err(|err| ShellError::Parse(err.to_string()))
}
