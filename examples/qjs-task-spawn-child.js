import * as std from "qjs:std";
import * as os from "qjs:os";

function readStdin() {
  const bytes = new Uint8Array(64);
  const count = os.read(0, bytes.buffer, 0, bytes.length);
  if (count < 0) {
    throw new Error("stdin read failed: " + count);
  }
  return Array.from(bytes.slice(0, count)).map((byte) => String.fromCharCode(byte)).join("");
}

std.out.puts(
  "child task "
    + std.loadFile("#task/self/id").trim()
    + " args "
    + scriptArgs.join("|")
    + " mode "
    + std.getenv("MODE")
    + " stdin "
    + readStdin().trimEnd()
    + "\n"
);
std.out.flush();
std.err.puts("child stderr mode " + std.getenv("MODE") + "\n");
std.err.flush();
std.exit(5);
