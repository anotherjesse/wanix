import * as os from "qjs:os";
import * as std from "qjs:std";

std.out.puts("sync\n");

os.setReadHandler(0, () => {
  const bytes = new Uint8Array(128);
  const n = os.read(0, bytes.buffer, 0, bytes.length);
  const text = Array.from(bytes.slice(0, n)).map((byte) => String.fromCharCode(byte)).join("");
  std.out.puts("handler: " + text + "\n");
  os.setReadHandler(0, null);
  std.out.flush();
});

std.out.flush();
