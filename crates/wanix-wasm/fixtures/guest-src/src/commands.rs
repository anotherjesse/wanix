mod compute;
mod files;
mod raw;

pub(crate) use files::copy_transform;

type CommandHandler = fn(&[String]);

const COMMANDS: &[(&str, CommandHandler)] = &[
    ("--list", list_dir_command),
    ("--rename", rename_command),
    ("--rmdir", rmdir_command),
    ("--symlink", symlink_command),
    ("--readlink", readlink_command),
    ("--tell", tell_command),
    ("--truncate", truncate_command),
    ("--pi", pi_command),
    ("--utime", utime_command),
    ("--echo", echo_command),
    ("--cat", cat_command),
    ("--gen", gen_command),
    ("--env", env_command),
];

pub(crate) fn run(args: &[String]) -> bool {
    let Some(name) = args.get(1).map(String::as_str) else {
        return false;
    };
    let Some((_, handler)) = COMMANDS.iter().find(|(command, _)| *command == name) else {
        return false;
    };
    handler(args);
    true
}

fn list_dir_command(args: &[String]) {
    files::list_dir(args.get(2).map_or("/", String::as_str));
}

fn rename_command(args: &[String]) {
    files::rename(
        args.get(2).map_or("", String::as_str),
        args.get(3).map_or("", String::as_str),
    );
}

fn rmdir_command(args: &[String]) {
    files::rmdir(args.get(2).map_or("", String::as_str));
}

fn symlink_command(args: &[String]) {
    raw::symlink(
        args.get(2).map_or("", String::as_str),
        args.get(3).map_or("", String::as_str),
    );
}

fn readlink_command(args: &[String]) {
    files::readlink(args.get(2).map_or("", String::as_str));
}

fn tell_command(args: &[String]) {
    raw::tell(
        args.get(2).map_or("", String::as_str),
        args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0),
    );
}

fn truncate_command(args: &[String]) {
    files::truncate(
        args.get(2).map_or("", String::as_str),
        args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0),
    );
}

fn pi_command(args: &[String]) {
    compute::pi(args.get(2).and_then(|s| s.parse().ok()));
}

fn utime_command(args: &[String]) {
    raw::utime(
        args.get(2).map_or("", String::as_str),
        args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0),
    );
}

fn echo_command(_args: &[String]) {
    files::echo();
}

/// Copies standard input to standard output (a `cat` with no file args), so the
/// shell's pipeline tests have a real stdin-reading consumer.
fn cat_command(_args: &[String]) {
    use std::io::{Read, Write};
    let mut input = Vec::new();
    if std::io::stdin().read_to_end(&mut input).is_ok() {
        let _ = std::io::stdout().write_all(&input);
    }
}

/// Streams `<bytes>` of a repeating pattern to stdout in small chunks, so the
/// shell's pipeline tests have a producer that writes more than a bounded
/// `#pipe` holds — it can only finish if a consumer drains concurrently.
fn gen_command(args: &[String]) {
    use std::io::Write;
    let total: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let chunk: Vec<u8> = (0..1024u32).map(|i| b'a' + (i % 26) as u8).collect();
    let mut stdout = std::io::stdout();
    let mut written = 0;
    while written < total {
        let len = chunk.len().min(total - written);
        if stdout.write_all(&chunk[..len]).is_err() {
            std::process::exit(1);
        }
        written += len;
    }
}

/// Prints the guest's environment as sorted `KEY=VALUE` lines, so the shell's
/// tests can prove a child inherits the shell's exported environment.
fn env_command(_args: &[String]) {
    let mut vars: Vec<(String, String)> = std::env::vars().collect();
    vars.sort();
    for (key, value) in vars {
        println!("{key}={value}");
    }
}
