import * as std from "qjs:std";
import * as os from "qjs:os";

const bytes = new Uint8Array(1024);
const count = os.read(0, bytes.buffer, 0, bytes.length);
const input = Array.from(bytes.slice(0, count))
  .map((byte) => String.fromCharCode(byte))
  .join("");

std.out.puts("stdin: " + input.trimEnd() + "\n");
std.out.puts("task id: " + std.loadFile("#task/self/id").trim() + "\n");
std.out.flush();
