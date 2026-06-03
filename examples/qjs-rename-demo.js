import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("old.txt", "hello from rename");
std.out.puts("rename: " + os.rename("old.txt", "renamed.txt") + "\n");

const oldFd = os.open("old.txt", os.O_RDONLY);
if (oldFd >= 0) {
  os.close(oldFd);
}
std.out.puts("old missing: " + (oldFd < 0) + "\n");
std.out.puts("message: " + std.loadFile("renamed.txt") + "\n");
std.out.flush();
