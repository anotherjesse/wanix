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

    CmdArgParser::new(command).parse().map(Some)
}

#[derive(Clone, Copy)]
enum CmdQuote {
    Single,
    Double,
}

impl CmdQuote {
    fn from_char(ch: char) -> Option<Self> {
        match ch {
            '\'' => Some(Self::Single),
            '"' => Some(Self::Double),
            _ => None,
        }
    }
}

struct CmdArgParser<'a> {
    chars: std::str::Chars<'a>,
    args: Vec<String>,
    current: String,
    in_word: bool,
    quote: Option<CmdQuote>,
}

impl<'a> CmdArgParser<'a> {
    fn new(command: &'a str) -> Self {
        Self {
            chars: command.chars(),
            args: Vec::new(),
            current: String::new(),
            in_word: false,
            quote: None,
        }
    }

    fn parse(mut self) -> FsResult<Vec<String>> {
        while let Some(ch) = self.chars.next() {
            match self.quote {
                Some(CmdQuote::Single) => self.push_single_quoted(ch),
                Some(CmdQuote::Double) => self.push_double_quoted(ch)?,
                None => self.push_unquoted(ch)?,
            }
        }

        self.finish()
    }

    fn push_single_quoted(&mut self, ch: char) {
        if ch == '\'' {
            self.quote = None;
        } else {
            self.current.push(ch);
        }
    }

    fn push_double_quoted(&mut self, ch: char) -> FsResult<()> {
        if ch == '"' {
            self.quote = None;
        } else if ch == '\\' {
            let next = self.next_escaped_char()?;
            if next != '"' && next != '\\' {
                self.current.push('\\');
            }
            self.current.push(next);
        } else {
            self.current.push(ch);
        }
        Ok(())
    }

    fn push_unquoted(&mut self, ch: char) -> FsResult<()> {
        if ch.is_whitespace() {
            self.finish_word();
        } else if let Some(quote) = CmdQuote::from_char(ch) {
            self.quote = Some(quote);
            self.in_word = true;
        } else if ch == '\\' {
            let next = self.next_escaped_char()?;
            self.current.push(next);
            self.in_word = true;
        } else {
            self.current.push(ch);
            self.in_word = true;
        }
        Ok(())
    }

    fn next_escaped_char(&mut self) -> FsResult<char> {
        self.chars
            .next()
            .ok_or_else(|| FsError::Other("unterminated escape in task cmd".to_owned()))
    }

    fn finish_word(&mut self) {
        if self.in_word {
            self.args.push(std::mem::take(&mut self.current));
            self.in_word = false;
        }
    }

    fn finish(mut self) -> FsResult<Vec<String>> {
        if self.quote.is_some() {
            return Err(FsError::Other("unterminated quote in task cmd".to_owned()));
        }
        self.finish_word();
        Ok(self.args)
    }
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
