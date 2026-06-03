import * as std from "qjs:std";
import * as os from "qjs:os";

function stringFromBytes(bytes, count) {
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

function escaped(text) {
  return text.replace(/\n/g, "\\n");
}

const winch = os.open("#term/1/winch", os.O_RDONLY);
const bytes = new Uint8Array(32);

std.out.puts("winch armed\n");

os.setReadHandler(winch, () => {
  const count = os.read(winch, bytes.buffer, 0, bytes.length);
  if (count <= 0) {
    throw new Error("winch read should have resize bytes, got " + count);
  }
  std.out.puts("winch " + escaped(stringFromBytes(bytes, count)) + "\n");
  os.setReadHandler(winch, null);
  os.close(winch);
  std.out.flush();
});

std.out.flush();
