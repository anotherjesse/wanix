import * as os from "qjs:os";
import * as std from "qjs:std";

std.out.puts("sync\n");

os.sleepAsync(0).then(() => {
  std.out.puts("sleepAsync\n");
  std.out.flush();
});

std.out.flush();
