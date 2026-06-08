//! Prompt rendering (pure).
//!
//! Renders a PS1 string against [`ShellState`] using `brush_parser::prompt`. It
//! is intentionally side-effect free and has no interactive loop yet — the
//! non-interactive `-c` path never prints a prompt. This is the piece a future
//! interactive REPL calls, so the cwd-aware prompt is ready and tested now.
//! Escapes we do not model (history/command numbers, time/date, job count, TTY,
//! non-printing markers) render as empty rather than panicking.

use brush_parser::prompt::{self, PromptPiece};

use crate::state::ShellState;

/// The default prompt: the working directory then `" $ "` (e.g. `/work $ `).
pub const DEFAULT_PS1: &str = "\\w $ ";

/// Renders `ps1` against `state`. A malformed PS1 falls back to its literal text.
#[must_use]
pub fn render(ps1: &str, state: &ShellState) -> String {
    let Ok(pieces) = prompt::parse(ps1) else {
        return ps1.to_owned();
    };
    let mut out = String::new();
    for piece in pieces {
        render_piece(&piece, state, &mut out);
    }
    out
}

fn render_piece(piece: &PromptPiece, state: &ShellState, out: &mut String) {
    match piece {
        PromptPiece::Literal(text) => out.push_str(text),
        PromptPiece::AsciiCharacter(code) => {
            if let Some(ch) = char::from_u32(*code) {
                out.push(ch);
            }
        }
        PromptPiece::Backslash => out.push('\\'),
        PromptPiece::Newline => out.push('\n'),
        PromptPiece::CarriageReturn => out.push('\r'),
        PromptPiece::DollarOrPound => out.push('$'),
        PromptPiece::CurrentWorkingDirectory { basename, .. } => {
            let cwd = state.cwd_display();
            if *basename {
                out.push_str(cwd.rsplit('/').find(|seg| !seg.is_empty()).unwrap_or("/"));
            } else {
                out.push_str(&cwd);
            }
        }
        PromptPiece::CurrentUser => out.push_str(state.env_get("USER").unwrap_or("user")),
        PromptPiece::Hostname {
            only_up_to_first_dot,
        } => {
            let host = state.env_get("HOSTNAME").unwrap_or("wanix");
            let host = if *only_up_to_first_dot {
                host.split('.').next().unwrap_or(host)
            } else {
                host
            };
            out.push_str(host);
        }
        PromptPiece::ShellBaseName => out.push_str("wsh"),
        // Everything else (history/command numbers, time/date, jobs, tty,
        // bell/escape, non-printing markers) renders empty for now.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_at(cwd: &str) -> ShellState {
        let mut state = ShellState::new();
        state.set_cwd(cwd.to_owned());
        state
    }

    #[test]
    fn default_ps1_shows_cwd() {
        assert_eq!(render(DEFAULT_PS1, &state_at("work")), "/work $ ");
        assert_eq!(render(DEFAULT_PS1, &ShellState::new()), "/ $ ");
    }

    #[test]
    fn basename_escape_uses_last_segment() {
        assert_eq!(render("\\W$ ", &state_at("work/sub")), "sub$ ");
        assert_eq!(render("\\W", &ShellState::new()), "/");
    }

    #[test]
    fn user_and_host_come_from_env() {
        let mut state = state_at("work");
        state.env_set("USER".into(), "jesse".into());
        state.env_set("HOSTNAME".into(), "box.local".into());
        assert_eq!(render("\\u@\\h:\\w$ ", &state), "jesse@box:/work$ ");
    }

    #[test]
    fn unsupported_escapes_render_empty_without_panicking() {
        // \t (time), \! (history) are not modeled; they vanish, no panic.
        assert_eq!(render("\\t\\!\\$ ", &ShellState::new()), "$ ");
    }

    #[test]
    fn malformed_ps1_falls_back_to_literal() {
        // A lone trailing backslash is not a valid escape; render literally.
        let rendered = render("weird\\", &ShellState::new());
        assert!(rendered.contains("weird"));
    }
}
