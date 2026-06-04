use std::sync::Arc;

use super::{CRATE_PURPOSE, TermDevice};
use wanix_fs::{FileSystem, NormalizedPath, OpenOptions};
use wanix_task::{Fd, Task, TaskSpec};
use wanix_vfs::{BindOptions, Namespace};

#[test]
fn purpose_is_declared() {
    assert!(!CRATE_PURPOSE.is_empty());
}

#[test]
fn new_allocates_incrementing_resources() {
    let terms = TermDevice::new();

    let first = read_file(&terms, "new");
    let second = read_file(&terms, "new");

    assert_eq!(first, b"1\n");
    assert_eq!(second, b"2\n");
    assert_eq!(read_file(&terms, "1/id"), b"1\n");

    let entries = terms
        .read_dir(&NormalizedPath::new(".").unwrap())
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(entries, ["new", "1", "2"]);
    let resource_entries = terms
        .read_dir(&NormalizedPath::new("1").unwrap())
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(resource_entries, ["ctl", "data", "id", "program", "winch"]);
}

#[test]
fn data_and_program_are_cross_connected() {
    let terms = TermDevice::new();
    let id = terms.alloc().unwrap();
    let mut data = terms
        .open(
            &NormalizedPath::new(format!("{id}/data")).unwrap(),
            OpenOptions::read_write(),
        )
        .unwrap();
    let mut program = terms
        .open(
            &NormalizedPath::new(format!("{id}/program")).unwrap(),
            OpenOptions::read_write(),
        )
        .unwrap();

    data.write(b"input").unwrap();
    assert!(program.read_ready().unwrap());
    assert_eq!(read_exact(&mut *program, 5), b"input");
    assert!(!program.read_ready().unwrap());

    program.write(b"line\nnext").unwrap();
    assert!(data.read_ready().unwrap());
    assert_eq!(read_exact(&mut *data, 10), b"line\r\nnext");
    assert!(!data.read_ready().unwrap());

    program.write(b"\r").unwrap();
    program.write(b"\n").unwrap();
    assert_eq!(read_exact(&mut *data, 2), b"\r\n");
}

#[test]
fn winch_broadcasts_to_open_readers() {
    let terms = TermDevice::new();
    let id = terms.alloc().unwrap();
    let winch_path = NormalizedPath::new(format!("{id}/winch")).unwrap();
    let mut first = terms.open(&winch_path, OpenOptions::read()).unwrap();
    let mut second = terms.open(&winch_path, OpenOptions::read()).unwrap();
    let mut writer = terms
        .open(
            &winch_path,
            OpenOptions {
                write: true,
                ..OpenOptions::default()
            },
        )
        .unwrap();

    assert!(!first.read_ready().unwrap());
    assert!(!second.read_ready().unwrap());
    assert!(!writer.read_ready().unwrap());

    writer.write(b"80x24").unwrap();

    assert!(first.read_ready().unwrap());
    assert!(second.read_ready().unwrap());
    assert_eq!(read_exact(&mut *first, 5), b"80x24");
    assert!(!first.read_ready().unwrap());
    assert!(second.read_ready().unwrap());
    assert_eq!(read_exact(&mut *second, 5), b"80x24");
    assert!(!second.read_ready().unwrap());
}

#[test]
fn ctl_close_removes_resource_and_invalidates_open_handles() {
    let terms = TermDevice::new();
    let id = terms.alloc().unwrap();
    let mut data = terms
        .open(
            &NormalizedPath::new(format!("{id}/data")).unwrap(),
            OpenOptions::read_write(),
        )
        .unwrap();
    let mut program = terms
        .open(
            &NormalizedPath::new(format!("{id}/program")).unwrap(),
            OpenOptions::read_write(),
        )
        .unwrap();
    let mut ctl = terms
        .open(
            &NormalizedPath::new(format!("{id}/ctl")).unwrap(),
            OpenOptions {
                write: true,
                ..OpenOptions::default()
            },
        )
        .unwrap();

    ctl.write(b"clo").unwrap();
    program.write(b"still open\n").unwrap();
    assert_eq!(read_exact(&mut *data, 12), b"still open\r\n");

    ctl.write(b"se\n").unwrap();

    assert!(matches!(
        terms.open(
            &NormalizedPath::new(format!("{id}/data")).unwrap(),
            OpenOptions::read()
        ),
        Err(wanix_fs::FsError::NotFound)
    ));
    assert_eq!(
        data.write(b"input").unwrap_err(),
        wanix_fs::FsError::InvalidFd
    );
    assert_eq!(
        program.write(b"output").unwrap_err(),
        wanix_fs::FsError::InvalidFd
    );
    let entries = terms
        .read_dir(&NormalizedPath::new(".").unwrap())
        .unwrap()
        .into_iter()
        .map(|entry| entry.name().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(entries, ["new"]);
}

#[test]
fn task_fd_can_bind_to_terminal_program_file() {
    let terms = Arc::new(TermDevice::new());
    let id = terms.alloc().unwrap();
    let task = Task::new(
        wanix_task::TaskId::new(1),
        TaskSpec::new("shell.js").unwrap(),
        Namespace::new(),
    );
    task.bind(terms.clone(), ".", "term", BindOptions::default())
        .unwrap();
    task.bind_fd_from_namespace(format!("term/{id}/program"), Fd::STDOUT)
        .unwrap();

    task.write_fd(Fd::STDOUT, b"hello\n").unwrap();

    let mut data = terms
        .open(
            &NormalizedPath::new(format!("{id}/data")).unwrap(),
            OpenOptions::read(),
        )
        .unwrap();
    assert_eq!(read_exact(&mut *data, 7), b"hello\r\n");
}

fn read_file(fs: &dyn FileSystem, path: &str) -> Vec<u8> {
    let mut file = fs
        .open(&NormalizedPath::new(path).unwrap(), OpenOptions::read())
        .unwrap();
    let mut out = Vec::new();
    let mut buf = [0; 16];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&buf[..n]);
    }
}

fn read_exact(file: &mut dyn wanix_fs::File, len: usize) -> Vec<u8> {
    let mut out = vec![0; len];
    let n = file.read(&mut out).unwrap();
    assert_eq!(n, len);
    out
}
