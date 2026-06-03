import * as std from "qjs:std";

const id = std.loadFile("#task/self/id").trim();
std.out.puts("after task: " + id + "\n");
std.out.puts("after cmd: " + Wanix.cmd() + "\n");
std.out.puts("after argv: " + scriptArgs.join("|") + "\n");
std.out.puts("after wasi env: " + std.getenv("MODE") + "\n");
std.out.puts("after wanix env: " + Wanix.env("MODE") + "\n");
std.out.puts("snapshot argv: " + globalThis.beforeArgs + "\n");
std.out.puts("snapshot env: " + globalThis.beforeMode + "\n");
std.out.flush();
std.exit(6);
