import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

let chunks = 0;
const bytes = new Uint8Array(4);

std.out.puts("terminal task: " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts("terminal id: " + std.loadFile("#term/1/id").trim() + "\n");
std.out.puts("waiting for terminal input\n");

os.setReadHandler(0, () => {
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("post-eval terminal read failed: " + count);
  }
  chunks += 1;
  std.out.puts("post-eval chunk " + chunks + ": " + stringFromBytes(bytes, count) + "\n");
  if (chunks >= 2 || count === 0) {
    os.setReadHandler(0, null);
    std.err.puts("post-eval handler done\n");
    std.out.flush();
    std.err.flush();
  }
});

std.out.flush();
