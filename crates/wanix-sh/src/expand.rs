//! Word expansion (honest-scope).
//!
//! Runs at execution time (per stage, against current state) over
//! `brush_parser::word` pieces, so quoting is handled once and a command sees
//! same-line state changes. Supported: `$VAR` / `${VAR}` parameter
//! expansion from the shell environment, `$?` (last exit status), quote removal,
//! and escape sequences. Globbing is **not** performed — pattern characters pass
//! through literally (so e.g. a `jaq` filter `.[]` survives). Command
//! substitution, arithmetic, `${VAR:-default}` and friends, tilde, and ANSI-C
//! quoting are recognized and reported as [`ShellError::Unsupported`].

use brush_parser::ParserOptions;
use brush_parser::word::{
    Parameter, ParameterExpr, SpecialParameter, WordPiece, WordPieceWithSource, parse,
};

use crate::error::{ShellError, ShellResult};
use crate::state::ShellState;

/// Expands a raw word into its final string using `state`'s environment.
///
/// # Errors
///
/// Returns [`ShellError::Parse`] if the word is malformed, or
/// [`ShellError::Unsupported`] for an expansion construct not yet implemented.
pub fn expand_word(raw: &str, state: &ShellState) -> ShellResult<String> {
    let pieces =
        parse(raw, &ParserOptions::default()).map_err(|err| ShellError::Parse(err.to_string()))?;
    let mut out = String::new();
    expand_pieces(&pieces, state, &mut out)?;
    Ok(out)
}

fn expand_pieces(
    pieces: &[WordPieceWithSource],
    state: &ShellState,
    out: &mut String,
) -> ShellResult<()> {
    for piece in pieces {
        expand_piece(&piece.piece, state, out)?;
    }
    Ok(())
}

fn expand_piece(piece: &WordPiece, state: &ShellState, out: &mut String) -> ShellResult<()> {
    match piece {
        WordPiece::Text(text) | WordPiece::SingleQuotedText(text) => out.push_str(text),
        WordPiece::DoubleQuotedSequence(inner) => expand_pieces(inner, state, out)?,
        WordPiece::EscapeSequence(seq) => out.push_str(seq.strip_prefix('\\').unwrap_or(seq)),
        WordPiece::ParameterExpansion(expr) => out.push_str(&expand_parameter(expr, state)?),
        WordPiece::TildeExpansion(_) => return Err(unsupported("tilde expansion (~)")),
        WordPiece::CommandSubstitution(_) | WordPiece::BackquotedCommandSubstitution(_) => {
            return Err(unsupported("command substitution"));
        }
        WordPiece::ArithmeticExpression(_) => return Err(unsupported("arithmetic expansion")),
        WordPiece::AnsiCQuotedText(_) => return Err(unsupported("ANSI-C quoting ($'...')")),
        WordPiece::GettextDoubleQuotedSequence(_) => return Err(unsupported("gettext quoting")),
    }
    Ok(())
}

fn expand_parameter(expr: &ParameterExpr, state: &ShellState) -> ShellResult<String> {
    match expr {
        ParameterExpr::Parameter {
            parameter,
            indirect: false,
        } => match parameter {
            // Unset variables expand to empty (POSIX), matching bash without `-u`.
            Parameter::Named(name) => Ok(state.env_get(name).unwrap_or("").to_owned()),
            Parameter::Special(SpecialParameter::LastExitStatus) => {
                Ok(state.last_status().to_string())
            }
            _ => Err(unsupported("this parameter form")),
        },
        _ => Err(unsupported(
            "parameter expansion modifiers (e.g. ${VAR:-default})",
        )),
    }
}

fn unsupported(what: &str) -> ShellError {
    ShellError::Unsupported(what.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(pairs: &[(&str, &str)]) -> ShellState {
        let mut state = ShellState::new();
        for (key, value) in pairs {
            state.env_set((*key).to_owned(), (*value).to_owned());
        }
        state
    }

    fn expand(raw: &str, state: &ShellState) -> ShellResult<String> {
        expand_word(raw, state)
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(expand("echo", &ShellState::new()).unwrap(), "echo");
    }

    #[test]
    fn named_parameter_expands_from_env() {
        let state = state_with(&[("NAME", "wanix")]);
        assert_eq!(expand("$NAME", &state).unwrap(), "wanix");
        assert_eq!(expand("${NAME}", &state).unwrap(), "wanix");
    }

    #[test]
    fn unset_parameter_is_empty() {
        assert_eq!(expand("$MISSING", &ShellState::new()).unwrap(), "");
    }

    #[test]
    fn last_status_expands() {
        let mut state = ShellState::new();
        state.set_last_status(7);
        assert_eq!(expand("$?", &state).unwrap(), "7");
    }

    #[test]
    fn single_quotes_are_literal() {
        let state = state_with(&[("NAME", "wanix")]);
        assert_eq!(expand("'$NAME'", &state).unwrap(), "$NAME");
    }

    #[test]
    fn double_quotes_expand_parameters() {
        let state = state_with(&[("NAME", "wanix")]);
        assert_eq!(expand("\"hi $NAME\"", &state).unwrap(), "hi wanix");
    }

    #[test]
    fn glob_metacharacters_pass_through_literally() {
        // A jaq filter like `.[]` must survive unchanged (no globbing).
        assert_eq!(expand(".[]", &ShellState::new()).unwrap(), ".[]");
    }

    #[test]
    fn unsupported_constructs_are_honest() {
        let state = ShellState::new();
        assert!(matches!(
            expand("$(echo hi)", &state).unwrap_err(),
            ShellError::Unsupported(_)
        ));
        assert!(matches!(
            expand("$((1+1))", &state).unwrap_err(),
            ShellError::Unsupported(_)
        ));
        assert!(matches!(
            expand("${VAR:-default}", &state).unwrap_err(),
            ShellError::Unsupported(_)
        ));
    }
}
