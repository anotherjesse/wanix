import * as std from "qjs:std";

const id = std.loadFile("#task/self/id").trim();
const output = "restored task " + id + " saw " + globalThis.snapshotMessage;
std.writeFile("host/restored-output.txt", output);
std.out.puts("after task: " + id + "\n");
std.out.puts("host output: " + std.loadFile("host/restored-output.txt") + "\n");
std.out.flush();
std.exit(8);
