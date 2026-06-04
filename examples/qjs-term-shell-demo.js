import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

let pending = "";
let running = true;
const rawInput = std.getenv("WANIX_QJS_SHELL_RAW") === "1";

function prompt() {
  if (running) {
    std.out.puts("$ ");
  }
}

function runCommand(line) {
  const trimmed = line.trim();
  if (trimmed === "") {
    prompt();
    return;
  }
  if (trimmed === "exit") {
    running = false;
    os.setReadHandler(0, null);
    std.out.puts("bye\n");
    std.out.flush();
    std.exit(0);
    return;
  }
  if (trimmed === "id") {
    std.out.puts(std.loadFile("#task/self/id").trim() + "\n");
    prompt();
    return;
  }
  if (trimmed === "pwd") {
    std.out.puts(std.loadFile("#task/self/dir").trim() + "\n");
    prompt();
    return;
  }
  if (trimmed.startsWith("echo ")) {
    std.out.puts(trimmed.slice(5) + "\n");
    prompt();
    return;
  }
  if (trimmed.startsWith("later ")) {
    const message = trimmed.slice(6);
    std.out.puts("scheduled\n");
    os.setTimeout(() => {
      if (!running) {
        return;
      }
      std.out.puts("later: " + message + "\n");
      prompt();
      std.out.flush();
    }, 1);
    return;
  }
  std.out.puts("unknown: " + trimmed + "\n");
  prompt();
}

function handleRawByte(byte) {
  if (byte === 0x04 && pending.length === 0) {
    runCommand("exit");
    return;
  }
  if (byte === 0x08 || byte === 0x7f) {
    if (pending.length > 0) {
      pending = pending.slice(0, -1);
      std.out.puts("\x08 \x08");
    }
    return;
  }
  if (byte === 0x0d || byte === 0x0a) {
    std.out.puts("\n");
    const line = pending;
    pending = "";
    runCommand(line);
    return;
  }
  if (byte === 0x09 || byte >= 0x20) {
    const char = String.fromCharCode(byte);
    pending += char;
    std.out.puts(char);
  }
}

function handleRawInput(bytes, count) {
  for (let i = 0; i < count; i++) {
    if (!running) {
      return;
    }
    handleRawByte(bytes[i]);
  }
}

function handleLineInput(bytes, count) {
  pending += stringFromBytes(bytes, count);
  let newline;
  while ((newline = pending.indexOf("\n")) >= 0) {
    const line = pending.slice(0, newline);
    pending = pending.slice(newline + 1);
    runCommand(line);
  }
}

std.out.puts("shell task: " + std.loadFile("#task/self/id").trim() + "\n");
prompt();

os.setReadHandler(0, () => {
  const bytes = new Uint8Array(256);
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("shell terminal read failed: " + count);
  }
  if (rawInput) {
    handleRawInput(bytes, count);
  } else {
    handleLineInput(bytes, count);
  }
  std.out.flush();
});

std.out.flush();
