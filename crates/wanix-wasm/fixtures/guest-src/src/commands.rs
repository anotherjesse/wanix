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
