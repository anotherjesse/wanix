import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("delete-me.txt", "remove me");
std.writeFile("keep.txt", "keep me");
os.remove("delete-me.txt");

const probe = os.open("delete-me.txt", os.O_RDONLY);
const deleted = probe < 0;
if (probe >= 0) {
  os.close(probe);
}

std.out.puts("deleted: " + deleted + "\n");
std.out.puts("kept: " + std.loadFile("keep.txt") + "\n");
std.out.flush();
