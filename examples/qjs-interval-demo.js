import * as os from "qjs:os";
import * as std from "qjs:std";

let count = 0;

std.out.puts("sync\n");

const interval = os.setInterval(() => {
  count += 1;
  std.out.puts("tick " + count + "\n");
  if (count >= 3) {
    os.clearInterval(interval);
    std.out.flush();
  }
}, 1);

std.out.flush();
