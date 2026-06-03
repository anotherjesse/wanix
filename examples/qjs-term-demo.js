import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

const bytes = new Uint8Array(128);
const count = os.read(0, bytes.buffer, 0, bytes.length);
const input = stringFromBytes(bytes, count).trimEnd();

std.out.puts("terminal task: " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts("terminal id: " + std.loadFile("#term/1/id").trim() + "\n");
std.out.puts("terminal input: " + input + "\n");
std.out.flush();
std.err.puts("terminal stderr: same screen\n");
std.err.flush();
