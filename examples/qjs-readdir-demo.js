import * as std from "qjs:std";
import * as os from "qjs:os";

function visible(path) {
  const [entries, err] = os.readdir(path);
  if (err !== 0) {
    throw new Error("readdir " + path + ": " + err);
  }
  return entries.filter((name) => name !== "." && name !== "..").sort().join(",");
}

os.mkdir("listing", 0o777);
os.mkdir("listing/nested", 0o777);
std.writeFile("listing/a.txt", "a");
std.writeFile("listing/b.txt", "b");

std.out.puts("listing: " + visible("listing") + "\n");
std.out.puts("nested: " + visible("listing/nested") + "\n");
std.out.flush();
