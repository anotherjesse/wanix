import * as std from "qjs:std";

const id = std.loadFile("#task/self/id").trim();
const taskMode = std.loadFile("#task/self/env").trim().replace(/^MODE=/, "");
std.out.puts("after task: " + id + "\n");
std.out.puts("after cmd: " + std.loadFile("#task/self/cmd").trim() + "\n");
std.out.puts("after argv: " + scriptArgs.join("|") + "\n");
std.out.puts("after wasi env: " + std.getenv("MODE") + "\n");
std.out.puts("after task env: " + taskMode + "\n");
std.out.puts("snapshot argv: " + globalThis.beforeArgs + "\n");
std.out.puts("snapshot env: " + globalThis.beforeMode + "\n");
std.out.flush();
std.exit(6);
