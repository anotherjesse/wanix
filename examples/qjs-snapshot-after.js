import * as std from "qjs:std";

const id = std.loadFile("#task/self/id").trim();
std.out.puts("after task: " + id + "\n");
std.out.puts("vm state: " + globalThis.snapshotMessage + "\n");
std.out.puts("namespace: " + std.loadFile("snapshot-note.txt") + "\n");
std.out.flush();
std.exit(7);
