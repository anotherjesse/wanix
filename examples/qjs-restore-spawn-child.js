import * as std from "qjs:std";

const id = std.loadFile("#task/self/id").trim();
std.out.puts("restore child task: " + id + "\n");
std.out.puts("restore child argv: " + scriptArgs.join("|") + "\n");
std.out.puts("restore child env: " + std.getenv("MODE") + "\n");
std.out.puts("restore child note: " + std.loadFile("restore-child-note.txt") + "\n");
std.out.puts("restore child snapshot global: " + typeof globalThis.snapshotMessage + "\n");
std.out.flush();
std.err.puts("restore child stderr: " + std.getenv("MODE") + "\n");
std.err.flush();
std.exit(5);
