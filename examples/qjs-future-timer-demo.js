import * as os from "qjs:os";
import * as std from "qjs:std";

std.out.puts("sync\n");

os.setTimeout(() => {
  std.out.puts("timeout\n");
  std.out.flush();
}, 1);

std.out.flush();
