import * as std from "qjs:std";
import * as os from "qjs:os";

std.writeFile("stamp.txt", "timestamped");
std.out.puts("utimes: " + os.utimes("stamp.txt", new Date(1000), new Date(2000)) + "\n");

const stat = os.stat("stamp.txt")[0];
std.out.puts("atime: " + stat.atime + "\n");
std.out.puts("mtime: " + stat.mtime + "\n");
std.out.puts("message: " + std.loadFile("stamp.txt") + "\n");
std.out.flush();
