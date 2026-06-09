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

// ---- bounded capacity ----------------------------------------------------

use std::num::NonZeroUsize;

use super::PipeCapacity;

fn bounded_device(bytes: usize) -> (PipeDevice, String) {
    let device =
        PipeDevice::with_capacity(PipeCapacity::Bounded(NonZeroUsize::new(bytes).unwrap()));
    let id = read_all(&mut device.open(&np("new"), OpenOptions::read()).unwrap());
    let data = format!("{}/data", id.trim());
    (device, data)
}

fn write_all(file: &mut Box<dyn File>, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let n = file.write(bytes).unwrap();
        assert!(n > 0, "pipe write made no progress");
        bytes = &bytes[n..];
    }
}

#[test]
fn full_pipe_short_writes_instead_of_overfilling() {
    let (device, data) = bounded_device(8);
    let mut writer = device.open(&np(&data), write_options()).unwrap();

    // Room for 8 bytes only: the 10-byte write is short.
    assert_eq!(writer.write(b"0123456789").unwrap(), 8);
    assert!(!writer.write_ready().unwrap(), "full pipe has no room");

    let mut reader = device.open(&np(&data), OpenOptions::read()).unwrap();
    let mut buf = [0u8; 16];
    assert_eq!(reader.read(&mut buf).unwrap(), 8);
    assert_eq!(&buf[..8], b"01234567");
    assert!(writer.write_ready().unwrap(), "drained pipe has room again");
}

#[test]
fn blocked_writer_completes_only_because_reader_drains_concurrently() {
    // The classic case: a producer writes several capacities worth of data into
    // a slow consumer. Under the old sequential model (producer must finish
    // before the consumer starts) this deadlocks; with a concurrent reader the
    // bounded pipe back-pressures the producer and everything flows.
    let (device, data) = bounded_device(64);
    let mut writer = device.open(&np(&data), write_options()).unwrap();
    let mut reader = device.open(&np(&data), OpenOptions::read()).unwrap();

    let payload: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
    let expected = payload.clone();
    let writer_thread = thread::spawn(move || {
        write_all(&mut writer, &payload);
        drop(writer);
    });

    let mut collected = Vec::new();
    let mut buf = [0u8; 48];
    loop {
        let n = reader.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        collected.extend_from_slice(&buf[..n]);
        assert!(
            collected.len() <= expected.len(),
            "reader saw more bytes than were written"
        );
    }
    writer_thread.join().unwrap();
    assert_eq!(collected, expected, "all bytes flow through the bound");
}

#[test]
fn writer_parks_until_a_reader_makes_room() {
    let (device, data) = bounded_device(4);
    let mut writer = device.open(&np(&data), write_options()).unwrap();
    write_all(&mut writer, b"full");

    let parked = thread::spawn(move || {
        // The buffer is full: this write blocks until the reader below drains.
        let n = writer.write(b"x").unwrap();
        assert_eq!(n, 1);
    });
    // Give the writer a moment to park (it must not return while full).
    thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        !parked.is_finished(),
        "write returned while the pipe was full"
    );

    let mut reader = device.open(&np(&data), OpenOptions::read()).unwrap();
    let mut buf = [0u8; 8];
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"full");
    parked.join().unwrap();
}

#[test]
fn default_capacity_bounds_a_single_write() {
    let device = PipeDevice::new();
    let id = read_all(&mut device.open(&np("new"), OpenOptions::read()).unwrap());
    let data = format!("{}/data", id.trim());
    let mut writer = device.open(&np(&data), write_options()).unwrap();

    let oversized = vec![0u8; PipeCapacity::DEFAULT_BYTES + 1];
    assert_eq!(
        writer.write(&oversized).unwrap(),
        PipeCapacity::DEFAULT_BYTES,
        "a write clamps at the default 64 KiB bound"
    );
    assert!(!writer.write_ready().unwrap());
}

#[test]
fn bounded_pipe_keeps_eof_on_last_writer_drop() {
    let (device, data) = bounded_device(4);
    let mut writer = device.open(&np(&data), write_options()).unwrap();
    assert_eq!(writer.write(b"hi").unwrap(), 2);
    drop(writer);

    let mut reader = device.open(&np(&data), OpenOptions::read()).unwrap();
    let mut buf = [0u8; 8];
    assert_eq!(reader.read(&mut buf).unwrap(), 2);
    assert_eq!(reader.read(&mut buf).unwrap(), 0, "EOF after last writer");
}

#[test]
fn write_after_last_reader_closes_is_a_broken_pipe_not_a_hang() {
    // A consumer that dies mid-stream must error the producer out of its
    // blocked write (Unix EPIPE), or whoever waits on the producer waits
    // forever.
    let (device, data) = bounded_device(4);
    let mut writer = device.open(&np(&data), write_options()).unwrap();
    let reader = device.open(&np(&data), OpenOptions::read()).unwrap();
    write_all(&mut writer, b"full");

    let producer = thread::spawn(move || {
        // Blocks on the full buffer until the reader drop below breaks it.
        writer.write(b"more")
    });
    thread::sleep(std::time::Duration::from_millis(50));
    drop(reader);
    assert!(
        producer.join().unwrap().is_err(),
        "blocked write should fail once the last reader is gone"
    );
}

#[test]
fn write_before_any_reader_opens_just_buffers() {
    let (device, data) = bounded_device(8);
    let mut writer = device.open(&np(&data), write_options()).unwrap();
    // No reader has ever opened: writes buffer instead of breaking.
    assert_eq!(writer.write(b"early").unwrap(), 5);

    let mut reader = device.open(&np(&data), OpenOptions::read()).unwrap();
    let mut buf = [0u8; 8];
    assert_eq!(reader.read(&mut buf).unwrap(), 5);
    assert_eq!(&buf[..5], b"early");
}

#[test]
fn unbounded_capacity_never_blocks_writes() {
    let device = PipeDevice::with_capacity(PipeCapacity::Unbounded);
    let id = read_all(&mut device.open(&np("new"), OpenOptions::read()).unwrap());
    let data = format!("{}/data", id.trim());
    let mut writer = device.open(&np(&data), write_options()).unwrap();

    let oversized = vec![7u8; PipeCapacity::DEFAULT_BYTES * 2];
    assert_eq!(writer.write(&oversized).unwrap(), oversized.len());
    assert!(writer.write_ready().unwrap());
}
