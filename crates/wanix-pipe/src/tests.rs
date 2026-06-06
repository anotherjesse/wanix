use std::thread;

use wanix_fs::{File, FileSystem, NormalizedPath, OpenOptions};

use super::{CRATE_PURPOSE, PipeDevice, modes};

fn np(path: &str) -> NormalizedPath {
    NormalizedPath::new(path).unwrap()
}

fn write_options() -> OpenOptions {
    OpenOptions {
        write: true,
        ..OpenOptions::default()
    }
}

fn read_all(file: &mut Box<dyn File>) -> String {
    let mut out = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    String::from_utf8(out).unwrap()
}

#[test]
fn purpose_is_declared() {
    assert!(!CRATE_PURPOSE.is_empty());
}

#[test]
fn new_allocates_incrementing_channels() {
    let device = PipeDevice::new();
    let mut first = device.open(&np("new"), OpenOptions::read()).unwrap();
    assert_eq!(read_all(&mut first), "1\n");
    let mut second = device.open(&np("new"), OpenOptions::read()).unwrap();
    assert_eq!(read_all(&mut second), "2\n");

    let mut id_file = device.open(&np("1/id"), OpenOptions::read()).unwrap();
    assert_eq!(read_all(&mut id_file), "1\n");

    let root: Vec<String> = device
        .read_dir(&np("."))
        .unwrap()
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    assert_eq!(root, ["new", "1", "2"]);

    let channel: Vec<String> = device
        .read_dir(&np("1"))
        .unwrap()
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    assert_eq!(channel, ["data", "id"]);
}

#[test]
fn metadata_modes_match_contract() {
    let device = PipeDevice::new();
    read_all(&mut device.open(&np("new"), OpenOptions::read()).unwrap());
    assert_eq!(device.metadata(&np(".")).unwrap().mode(), modes::DIRECTORY);
    assert_eq!(
        device.metadata(&np("new")).unwrap().mode(),
        modes::READ_ONLY_FILE
    );
    assert_eq!(device.metadata(&np("1")).unwrap().mode(), modes::DIRECTORY);
    assert_eq!(
        device.metadata(&np("1/id")).unwrap().mode(),
        modes::READ_ONLY_FILE
    );
    assert_eq!(
        device.metadata(&np("1/data")).unwrap().mode(),
        modes::STREAM_FILE
    );
}

#[test]
fn writer_bytes_round_trip_to_reader() {
    let device = PipeDevice::new();
    let id = read_all(&mut device.open(&np("new"), OpenOptions::read()).unwrap());
    let data = format!("{}/data", id.trim());

    let mut writer = device.open(&np(&data), write_options()).unwrap();
    let mut reader = device.open(&np(&data), OpenOptions::read()).unwrap();

    assert_eq!(writer.write(b"hello").unwrap(), 5);
    let mut buf = [0u8; 16];
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"hello");
}

#[test]
fn reader_sees_eof_after_last_writer_closes() {
    let device = PipeDevice::new();
    let id = read_all(&mut device.open(&np("new"), OpenOptions::read()).unwrap());
    let data = format!("{}/data", id.trim());

    let writer = device.open(&np(&data), write_options()).unwrap();
    let mut reader = device.open(&np(&data), OpenOptions::read()).unwrap();

    drop(writer);
    let mut buf = [0u8; 16];
    assert_eq!(reader.read(&mut buf).unwrap(), 0);
}

#[test]
fn concurrent_threads_round_trip_payload() {
    let device = PipeDevice::new();
    let id = read_all(&mut device.open(&np("new"), OpenOptions::read()).unwrap());
    let data = format!("{}/data", id.trim());

    let mut writer = device.open(&np(&data), write_options()).unwrap();
    let mut reader = device.open(&np(&data), OpenOptions::read()).unwrap();

    let payload: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    let expected = payload.clone();
    let writer_thread = thread::spawn(move || {
        for chunk in payload.chunks(256) {
            writer.write(chunk).unwrap();
        }
        drop(writer);
    });

    let mut collected = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = reader.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        collected.extend_from_slice(&buf[..n]);
    }
    writer_thread.join().unwrap();
    assert_eq!(collected, expected);
}
