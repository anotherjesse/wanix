import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

let pending = "";
let running = true;

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
  std.out.puts("unknown: " + trimmed + "\n");
  prompt();
}

std.out.puts("shell task: " + std.loadFile("#task/self/id").trim() + "\n");
prompt();

os.setReadHandler(0, () => {
  const bytes = new Uint8Array(256);
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("shell terminal read failed: " + count);
  }
  pending += stringFromBytes(bytes, count);
  let newline;
  while ((newline = pending.indexOf("\n")) >= 0) {
    const line = pending.slice(0, newline);
    pending = pending.slice(newline + 1);
    runCommand(line);
  }
  std.out.flush();
});

std.out.flush();
