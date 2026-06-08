//! Lowers the `brush-parser` AST into a flat, executable [`Plan`].
//!
//! This is where "scope honesty" lives: any AST node the executor does not yet
//! handle is turned into a clear [`ShellError::Unsupported`] rather than being
//! silently dropped. As the executor grows (pipelines, redirects, control flow),
//! the matching arms here move from "unsupported" to real lowering.

use brush_parser::{ast, unquote_str};

use crate::error::{ShellError, ShellResult};

/// A single simple command after word resolution: `argv[0]` is the command name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCmd {
    /// The resolved argument vector.
    pub argv: Vec<String>,
}

/// A lowered program: a sequence of simple commands run in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Commands to run in sequence (`;` / newline separated).
    pub commands: Vec<SimpleCmd>,
}

/// Lowers a parsed [`ast::Program`] into a [`Plan`].
///
/// # Errors
///
/// Returns [`ShellError::Unsupported`] for any construct outside the current
/// executor subset (pipelines, redirects, `&&`/`||`, control flow, …).
pub fn lower(program: &ast::Program) -> ShellResult<Plan> {
    let mut commands = Vec::new();
    for complete_command in &program.complete_commands {
        for item in &complete_command.0 {
            let and_or_list = &item.0;
            if !and_or_list.additional.is_empty() {
                return Err(ShellError::Unsupported("'&&' / '||' lists".into()));
            }
            lower_pipeline(&and_or_list.first, &mut commands)?;
        }
    }
    Ok(Plan { commands })
}

fn lower_pipeline(pipeline: &ast::Pipeline, out: &mut Vec<SimpleCmd>) -> ShellResult<()> {
    if pipeline.bang {
        return Err(ShellError::Unsupported("pipeline negation ('!')".into()));
    }
    if pipeline.seq.len() != 1 {
        return Err(ShellError::Unsupported("pipelines ('|')".into()));
    }
    out.push(lower_command(&pipeline.seq[0])?);
    Ok(())
}

fn lower_command(command: &ast::Command) -> ShellResult<SimpleCmd> {
    match command {
        ast::Command::Simple(simple) => lower_simple(simple),
        ast::Command::Compound(_, _) => Err(ShellError::Unsupported(
            "compound commands (if/for/while/case)".into(),
        )),
        ast::Command::Function(_) => Err(ShellError::Unsupported("function definitions".into())),
        ast::Command::ExtendedTest(_, _) => {
            Err(ShellError::Unsupported("'[[ ... ]]' tests".into()))
        }
    }
}

fn lower_simple(simple: &ast::SimpleCommand) -> ShellResult<SimpleCmd> {
    if simple
        .prefix
        .as_ref()
        .is_some_and(|prefix| !prefix.0.is_empty())
    {
        return Err(ShellError::Unsupported(
            "assignments or redirections before a command".into(),
        ));
    }

    let mut argv = Vec::new();
    if let Some(name) = &simple.word_or_name {
        argv.push(resolve_word(&name.value));
    }
    if let Some(suffix) = &simple.suffix {
        for item in &suffix.0 {
            match item {
                ast::CommandPrefixOrSuffixItem::Word(word) => argv.push(resolve_word(&word.value)),
                ast::CommandPrefixOrSuffixItem::IoRedirect(_) => {
                    return Err(ShellError::Unsupported("redirections".into()));
                }
                ast::CommandPrefixOrSuffixItem::AssignmentWord(_, _) => {
                    return Err(ShellError::Unsupported("inline assignments".into()));
                }
                ast::CommandPrefixOrSuffixItem::ProcessSubstitution(_, _) => {
                    return Err(ShellError::Unsupported("process substitution".into()));
                }
            }
        }
    }

    if argv.is_empty() {
        return Err(ShellError::Unsupported("empty command".into()));
    }
    Ok(SimpleCmd { argv })
}

/// Resolves a single word to its final value.
///
/// For now this only removes quotes via `brush_parser::unquote_str`. Real
/// expansion (variables, command substitution, globbing) is a later phase; words
/// containing those constructs pass through unexpanded today.
fn resolve_word(raw: &str) -> String {
    unquote_str(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parse_program;

    fn plan_of(input: &str) -> ShellResult<Plan> {
        lower(&parse_program(input).expect("parses"))
    }

    #[test]
    fn lowers_simple_command_with_args() {
        let plan = plan_of("echo hi there").expect("lowers");
        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].argv, vec!["echo", "hi", "there"]);
    }

    #[test]
    fn unquotes_words() {
        let plan = plan_of("echo \"hi there\" 'a b'").expect("lowers");
        assert_eq!(plan.commands[0].argv, vec!["echo", "hi there", "a b"]);
    }

    #[test]
    fn lowers_sequence_separated_by_semicolons() {
        let plan = plan_of("echo a; echo b").expect("lowers");
        assert_eq!(plan.commands.len(), 2);
        assert_eq!(plan.commands[1].argv, vec!["echo", "b"]);
    }

    #[test]
    fn pipelines_are_unsupported_honestly() {
        assert!(matches!(
            plan_of("echo a | wc").unwrap_err(),
            ShellError::Unsupported(_)
        ));
    }

    #[test]
    fn redirections_are_unsupported_honestly() {
        assert!(matches!(
            plan_of("echo a > f").unwrap_err(),
            ShellError::Unsupported(_)
        ));
    }

    #[test]
    fn control_flow_is_unsupported_honestly() {
        assert!(matches!(
            plan_of("for i in 1 2 3; do echo $i; done").unwrap_err(),
            ShellError::Unsupported(_)
        ));
    }
}
