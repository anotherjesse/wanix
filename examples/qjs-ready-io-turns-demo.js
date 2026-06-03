import * as os from "qjs:os";
import * as std from "qjs:std";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

let chunks = 0;
const bytes = new Uint8Array(3);

std.out.puts("sync\n");

os.setReadHandler(0, () => {
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("stdin read failed: " + count);
  }
  chunks += 1;
  std.out.puts("chunk " + chunks + ": " + stringFromBytes(bytes, count) + "\n");
  if (chunks >= 2 || count === 0) {
    os.setReadHandler(0, null);
    std.out.flush();
  }
});

std.out.flush();
