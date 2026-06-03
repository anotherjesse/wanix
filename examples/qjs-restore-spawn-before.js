import * as std from "qjs:std";

const id = std.loadFile("#task/self/id").trim();
globalThis.snapshotMessage = "preserved from task " + id;
std.writeFile("restore-parent-note.txt", "namespace from task " + id);
std.out.puts("before task: " + id + "\n");
std.out.flush();
