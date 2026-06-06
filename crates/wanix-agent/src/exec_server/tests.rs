use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};
use wanix_fs::MemFs;

use super::ExecServer;

fn server() -> ExecServer {
    ExecServer::new(Arc::new(MemFs::new()))
}

fn call(server: &mut ExecServer, method: &str, params: Value) -> Value {
    let lines = server.handle(&json!({ "id": 1, "method": method, "params": params }));
    lines[0]["result"].clone()
}

#[test]
fn fs_write_then_read_round_trips() {
    let mut server = server();
    call(
        &mut server,
        "fs/writeFile",
        json!({ "path": "/notes.txt", "dataBase64": BASE64.encode(b"banana") }),
    );
    let result = call(&mut server, "fs/readFile", json!({ "path": "/notes.txt" }));
    let bytes = BASE64
        .decode(result["dataBase64"].as_str().unwrap())
        .unwrap();
    assert_eq!(bytes, b"banana");
}

#[test]
fn process_echo_redirect_writes_a_wanix_file() {
    let mut server = server();
    call(
        &mut server,
        "process/start",
        json!({ "processId": "p1", "argv": ["/bin/zsh", "-lc", "echo banana > /notes.txt"], "cwd": "/" }),
    );
    let result = call(&mut server, "fs/readFile", json!({ "path": "/notes.txt" }));
    let bytes = BASE64
        .decode(result["dataBase64"].as_str().unwrap())
        .unwrap();
    assert_eq!(bytes, b"banana\n");
}

#[test]
fn process_cat_reads_a_wanix_file() {
    let mut server = server();
    call(
        &mut server,
        "fs/writeFile",
        json!({ "path": "/h.txt", "dataBase64": BASE64.encode(b"hello\n") }),
    );
    call(
        &mut server,
        "process/start",
        json!({ "processId": "p2", "argv": ["/bin/zsh", "-lc", "cat /h.txt"], "cwd": "/" }),
    );
    let result = call(
        &mut server,
        "process/read",
        json!({ "processId": "p2", "afterSeq": null }),
    );
    let chunk = result["chunks"][0]["chunk"].as_str().unwrap();
    assert_eq!(BASE64.decode(chunk).unwrap(), b"hello\n");
    assert_eq!(result["exitCode"].as_i64(), Some(0));
}

#[test]
fn read_directory_lists_entries() {
    let mut server = server();
    call(
        &mut server,
        "fs/writeFile",
        json!({ "path": "/a.txt", "dataBase64": BASE64.encode(b"x") }),
    );
    let result = call(&mut server, "fs/readDirectory", json!({ "path": "/" }));
    let entries = result["entries"].as_array().unwrap();
    assert!(entries.iter().any(|entry| entry["fileName"] == "a.txt"));
}

#[test]
fn unsupported_command_reports_a_clear_error() {
    let mut server = server();
    call(
        &mut server,
        "process/start",
        json!({ "processId": "p3", "argv": ["/bin/zsh", "-lc", "grep x /y | wc -l"], "cwd": "/" }),
    );
    let result = call(
        &mut server,
        "process/read",
        json!({ "processId": "p3", "afterSeq": null }),
    );
    assert_eq!(result["exitCode"].as_i64(), Some(1));
    let chunk = result["chunks"][0]["chunk"].as_str().unwrap();
    assert!(
        String::from_utf8(BASE64.decode(chunk).unwrap())
            .unwrap()
            .contains("unsupported")
    );
}
