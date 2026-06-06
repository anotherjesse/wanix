// Namespace filesystem convenience wrappers over qjs:std/qjs:os.
// readText/writeText use os.open/read/write/close directly so the same code
// reads ordinary files AND #task/... service files. Service files answer in a
// single bounded read; readBytes loops for ordinary files.

import * as std from "qjs:std";
import * as os from "qjs:os";
import { bytesFromString, stringFromBytes } from "./bytes.js";

const SERVICE_READ_BYTES = 4096;

export function readText(path) {
  const fd = os.open(path, os.O_RDONLY);
  if (fd < 0) throw new Error("open " + path + ": " + fd);
  try {
    const buf = new Uint8Array(SERVICE_READ_BYTES);
    const n = os.read(fd, buf.buffer, 0, buf.length);
    if (n < 0) throw new Error("read " + path + ": " + n);
    return stringFromBytes(buf, n);
  } finally {
    os.close(fd);
  }
}

export function readBytes(path) {
  const fd = os.open(path, os.O_RDONLY);
  if (fd < 0) throw new Error("open " + path + ": " + fd);
  const out = [];
  try {
    const buf = new Uint8Array(SERVICE_READ_BYTES);
    for (;;) {
      const n = os.read(fd, buf.buffer, 0, buf.length);
      if (n < 0) throw new Error("read " + path + ": " + n);
      if (n === 0) break;
      for (let i = 0; i < n; i++) out.push(buf[i]);
    }
  } finally {
    os.close(fd);
  }
  return new Uint8Array(out);
}

export function writeText(path, text) {
  const fd = os.open(path, os.O_WRONLY);
  if (fd < 0) throw new Error("open " + path + ": " + fd);
  try {
    const bytes = bytesFromString(text);
    const n = os.write(fd, bytes.buffer, 0, bytes.length);
    if (n !== bytes.length) {
      throw new Error("short write " + path + ": " + n + "/" + bytes.length);
    }
    return n;
  } finally {
    os.close(fd);
  }
}

export function createText(path, text) {
  const fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o666);
  if (fd < 0) throw new Error("open " + path + ": " + fd);
  try {
    const bytes = bytesFromString(text);
    return os.write(fd, bytes.buffer, 0, bytes.length);
  } finally {
    os.close(fd);
  }
}

export function list(path) {
  const [entries, err] = os.readdir(path);
  if (err !== 0) throw new Error("readdir " + path + ": " + err);
  return entries.filter((name) => name !== "." && name !== "..").sort();
}

export function exists(path) {
  const [, err] = os.stat(path);
  return err === 0;
}

export function mkdir(path, mode) {
  const err = os.mkdir(path, mode === undefined ? 0o777 : mode);
  if (err !== 0) throw new Error("mkdir " + path + ": " + err);
}

export function remove(path) {
  const err = os.remove(path);
  if (err !== 0) throw new Error("remove " + path + ": " + err);
}

export function writeFile(path, text) {
  std.writeFile(path, text);
}

export function loadFile(path) {
  return std.loadFile(path);
}
