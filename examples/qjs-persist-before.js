import * as std from "qjs:std";

const task = std.loadFile("#task/self/id").trim();
globalThis.persistedMessage = "vm from task " + task + " mode " + std.getenv("MODE");
std.writeFile("host/persist-before.txt", "host before task " + task);
std.out.puts("snapshot task: " + task + "\n");
std.out.flush();
