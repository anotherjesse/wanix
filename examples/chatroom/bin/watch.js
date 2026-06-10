// chatroom verb: stream the room's live feed to stdout, incrementally.
//
// Invoked from a shell that mounted the room: `room:watch`. Runs CONFINED
// (ADR 0007 §Confinement contract): only the room at /res is visible.
//
// /res/stream is the room's never-EOF subscription file: each blocking read
// parks until a new message line arrives (ADR 0010 blocking reads on the
// task's own thread), so this loop is a live tail, not a poll. EOF means the
// room went away; Ctrl-C in an interactive shell kills the verb task.
import * as std from "qjs:std";
import * as os from "qjs:os";

const fd = os.open("/res/stream", os.O_RDONLY);
if (fd < 0) {
  std.err.puts("watch: cannot open /res/stream\n");
  std.exit(1);
}
const buf = new Uint8Array(4096);
for (;;) {
  const n = os.read(fd, buf.buffer, 0, buf.length);
  if (n <= 0) break; // EOF: the room's guest exited or the mount closed.
  os.write(1, buf.buffer, 0, n);
}
os.close(fd);
