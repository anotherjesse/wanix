use wanix_fs::{FsError, FsResult};

/// Formats argv as task `cmd` text using the shell-quoted convention accepted by `#task/cmd`.
pub fn quote_cmd_argv<I, S>(argv: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    argv.into_iter()
        .map(|arg| quote_cmd_arg(arg.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn parse_cmd_argv(command: &str) -> FsResult<Option<Vec<String>>> {
    let command = command.trim();
    if command.is_empty() {
        return Ok(None);
    }

    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = command.chars();
    let mut quote = None;

    while let Some(ch) = chars.next() {
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    current.push(ch);
                }
            }
            Some('"') => {
                if ch == '"' {
                    quote = None;
                } else if ch == '\\' {
                    let next = chars.next().ok_or_else(|| {
                        FsError::Other("unterminated escape in task cmd".to_owned())
                    })?;
                    if next != '"' && next != '\\' {
                        current.push('\\');
                    }
                    current.push(next);
                } else {
                    current.push(ch);
                }
            }
            Some(_) => unreachable!("only shell quotes are installed"),
            None if ch.is_whitespace() => {
                if in_word {
                    args.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                in_word = true;
            }
            None if ch == '\\' => {
                let next = chars
                    .next()
                    .ok_or_else(|| FsError::Other("unterminated escape in task cmd".to_owned()))?;
                current.push(next);
                in_word = true;
            }
            None => {
                current.push(ch);
                in_word = true;
            }
        }
    }

    if quote.is_some() {
        return Err(FsError::Other("unterminated quote in task cmd".to_owned()));
    }
    if in_word {
        args.push(current);
    }
    Ok(Some(args))
}

fn quote_cmd_arg(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|ch| !ch.is_whitespace() && ch != '\'' && ch != '"' && ch != '\\')
    {
        return arg.to_owned();
    }

    let mut quoted = String::from("'");
    for ch in arg.chars() {
        if ch == '\'' {
            quoted.push_str("'\"'\"'");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}
