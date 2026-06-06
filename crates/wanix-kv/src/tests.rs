use wanix_fs::{FileSystem, FsError, NormalizedPath, OpenOptions};

use super::{CRATE_PURPOSE, KvDevice, modes};

fn np(path: &str) -> NormalizedPath {
    NormalizedPath::new(path).unwrap()
}

fn create_options() -> OpenOptions {
    OpenOptions {
        write: true,
        create: true,
        ..OpenOptions::default()
    }
}

fn write_key(kv: &KvDevice, key: &str, value: &[u8]) {
    let mut file = kv.open(&np(key), create_options()).unwrap();
    file.write(value).unwrap();
    drop(file);
}

fn read_key(kv: &KvDevice, key: &str) -> Vec<u8> {
    let mut file = kv.open(&np(key), OpenOptions::read()).unwrap();
    let mut out = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

#[test]
fn purpose_is_declared() {
    assert!(!CRATE_PURPOSE.is_empty());
}

#[test]
fn set_then_get_round_trips() {
    let kv = KvDevice::new();
    write_key(&kv, "foo", b"bar");
    assert_eq!(read_key(&kv, "foo"), b"bar");
}

#[test]
fn get_missing_key_is_not_found() {
    let kv = KvDevice::new();
    assert!(matches!(
        kv.open(&np("missing"), OpenOptions::read()),
        Err(FsError::NotFound)
    ));
    assert!(matches!(
        kv.metadata(&np("missing")),
        Err(FsError::NotFound)
    ));
}

#[test]
fn delete_removes_key() {
    let kv = KvDevice::new();
    write_key(&kv, "k", b"v");
    kv.remove_file(&np("k")).unwrap();
    assert!(matches!(
        kv.open(&np("k"), OpenOptions::read()),
        Err(FsError::NotFound)
    ));
    assert!(matches!(kv.remove_file(&np("k")), Err(FsError::NotFound)));
}

#[test]
fn read_dir_lists_keys_sorted() {
    let kv = KvDevice::new();
    write_key(&kv, "b", b"2");
    write_key(&kv, "a", b"1");
    write_key(&kv, "c", b"3");
    let keys: Vec<String> = kv
        .read_dir(&np("."))
        .unwrap()
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    assert_eq!(keys, ["a", "b", "c"]);
}

#[test]
fn overwrite_replaces_value_wholesale() {
    let kv = KvDevice::new();
    write_key(&kv, "x", b"longvalue");
    write_key(&kv, "x", b"hi");
    assert_eq!(read_key(&kv, "x"), b"hi");
}

#[test]
fn read_dir_on_key_is_not_directory() {
    let kv = KvDevice::new();
    write_key(&kv, "k", b"v");
    assert!(matches!(kv.read_dir(&np("k")), Err(FsError::NotDirectory)));
}

#[test]
fn open_root_is_directory() {
    let kv = KvDevice::new();
    assert!(matches!(
        kv.open(&np("."), OpenOptions::read()),
        Err(FsError::IsDirectory)
    ));
}

#[test]
fn create_makes_key_immediately_statable() {
    // Regression for the 9P Tlcreate flow, which stats the path immediately
    // after open (before the write handle is dropped and committed).
    let kv = KvDevice::new();
    let file = kv.open(&np("k"), create_options()).unwrap();
    let meta = kv.metadata(&np("k")).unwrap();
    assert_eq!(meta.mode(), modes::VALUE_FILE);
    drop(file);
    assert_eq!(read_key(&kv, "k"), b"");
}
