import * as std from "qjs:std";

const id = std.loadFile("#task/self/id").trim();
globalThis.beforeArgs = scriptArgs.join("|");
globalThis.beforeMode = std.getenv("MODE");
std.out.puts("before task: " + id + "\n");
std.out.puts("before cmd: " + Wanix.cmd() + "\n");
std.out.puts("before argv: " + scriptArgs.join("|") + "\n");
std.out.puts("before wasi env: " + std.getenv("MODE") + "\n");
std.out.puts("before wanix env: " + Wanix.env("MODE") + "\n");
std.out.flush();
