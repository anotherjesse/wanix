//! Lowers the `brush-parser` AST into a flat [`Plan`] of raw word stages.
//!
//! Lowering is structural only: it shapes pipelines and sequences and rejects
//! constructs the executor does not handle (honest [`ShellError::Unsupported`],
//! never a silent no-op). Word **values are kept raw** — quote removal and
//! `$VAR`/`$?` expansion happen at execution time (see [`crate::expand`]) so a
//! command sees state changes made earlier on the same line.

use brush_parser::ast;

use crate::error::{ShellError, ShellResult};

/// A single simple command as raw (unexpanded) word strings; `argv[0]` is the
/// command name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage {
    /// The raw argument words, expanded at execution time.
    pub argv: Vec<String>,
}

/// A pipeline: one or more [`Stage`]s connected stdout-to-stdin by pipes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    /// The stages, in left-to-right order.
    pub stages: Vec<Stage>,
}

/// How a following pipeline is gated on the previous one's exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connector {
    /// `&&`: run only if the previous pipeline succeeded (status 0).
    And,
    /// `||`: run only if the previous pipeline failed (status != 0).
    Or,
}

/// An and-or list: a first pipeline plus `&&`/`||`-connected followers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndOrList {
    /// The leading pipeline (always runs).
    pub first: Pipeline,
    /// Followers, each gated by its [`Connector`].
    pub rest: Vec<(Connector, Pipeline)>,
}

/// A lowered program: a sequence of and-or lists run in order (`;` / newline).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// And-or lists to run in sequence.
    pub lists: Vec<AndOrList>,
}

/// Lowers a parsed [`ast::Program`] into a [`Plan`].
///
/// # Errors
///
/// Returns [`ShellError::Unsupported`] for any construct outside the current
/// executor subset (redirects, `&&`/`||`, control flow, …).
pub fn lower(program: &ast::Program) -> ShellResult<Plan> {
    let mut lists = Vec::new();
    for complete_command in &program.complete_commands {
        for item in &complete_command.0 {
            lists.push(lower_and_or_list(&item.0)?);
        }
    }
    Ok(Plan { lists })
}

fn lower_and_or_list(and_or_list: &ast::AndOrList) -> ShellResult<AndOrList> {
    let first = lower_pipeline(&and_or_list.first)?;
    let mut rest = Vec::with_capacity(and_or_list.additional.len());
    for and_or in &and_or_list.additional {
        let (connector, pipeline) = match and_or {
            ast::AndOr::And(pipeline) => (Connector::And, pipeline),
            ast::AndOr::Or(pipeline) => (Connector::Or, pipeline),
        };
        rest.push((connector, lower_pipeline(pipeline)?));
    }
    Ok(AndOrList { first, rest })
}

fn lower_pipeline(pipeline: &ast::Pipeline) -> ShellResult<Pipeline> {
    if pipeline.bang {
        return Err(ShellError::Unsupported("pipeline negation ('!')".into()));
    }
    let mut stages = Vec::with_capacity(pipeline.seq.len());
    for command in &pipeline.seq {
        stages.push(lower_command(command)?);
    }
    Ok(Pipeline { stages })
}

fn lower_command(command: &ast::Command) -> ShellResult<Stage> {
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

fn lower_simple(simple: &ast::SimpleCommand) -> ShellResult<Stage> {
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
        argv.push(name.value.clone());
    }
    if let Some(suffix) = &simple.suffix {
        for item in &suffix.0 {
            match item {
                ast::CommandPrefixOrSuffixItem::Word(word) => argv.push(word.value.clone()),
                // After the command word, a `name=value` token is an ordinary
                // argument (e.g. `export A=1`, `echo A=1`), not an assignment.
                ast::CommandPrefixOrSuffixItem::AssignmentWord(_, word) => {
                    argv.push(word.value.clone());
                }
                ast::CommandPrefixOrSuffixItem::IoRedirect(_) => {
                    return Err(ShellError::Unsupported("redirections".into()));
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
    Ok(Stage { argv })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parse_program;

    fn plan_of(input: &str) -> ShellResult<Plan> {
        lower(&parse_program(input).expect("parses"))
    }

    fn argv(plan: &Plan, list: usize, stage: usize) -> &[String] {
        &plan.lists[list].first.stages[stage].argv
    }

    #[test]
    fn lowers_simple_command_with_args() {
        let plan = plan_of("echo hi there").expect("lowers");
        assert_eq!(plan.lists.len(), 1);
        assert_eq!(plan.lists[0].first.stages.len(), 1);
        assert_eq!(argv(&plan, 0, 0), ["echo", "hi", "there"]);
    }

    #[test]
    fn keeps_words_raw_for_execution_time_expansion() {
        // Quotes/`$VAR` survive lowering; expansion happens in exec.
        let plan = plan_of("echo \"hi there\" $NAME").expect("lowers");
        assert_eq!(argv(&plan, 0, 0), ["echo", "\"hi there\"", "$NAME"]);
    }

    #[test]
    fn lowers_sequence_separated_by_semicolons() {
        let plan = plan_of("echo a; echo b").expect("lowers");
        assert_eq!(plan.lists.len(), 2);
        assert_eq!(argv(&plan, 1, 0), ["echo", "b"]);
    }

    #[test]
    fn lowers_a_pipeline_into_stages() {
        let plan = plan_of("echo hi | wc -c | cat").expect("lowers");
        assert_eq!(plan.lists.len(), 1);
        assert_eq!(plan.lists[0].first.stages.len(), 3);
        assert_eq!(argv(&plan, 0, 1), ["wc", "-c"]);
    }

    #[test]
    fn lowers_and_or_connectors() {
        let plan = plan_of("true && echo ok || echo no").expect("lowers");
        assert_eq!(plan.lists.len(), 1);
        let list = &plan.lists[0];
        assert_eq!(list.first.stages[0].argv, ["true"]);
        assert_eq!(list.rest.len(), 2);
        assert_eq!(list.rest[0].0, Connector::And);
        assert_eq!(list.rest[1].0, Connector::Or);
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
