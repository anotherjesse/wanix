import * as std from "qjs:std";

const id = std.loadFile("#task/self/id").trim();
const taskMode = std.loadFile("#task/self/env").trim().replace(/^MODE=/, "");
globalThis.beforeArgs = scriptArgs.join("|");
globalThis.beforeMode = std.getenv("MODE");
std.out.puts("before task: " + id + "\n");
std.out.puts("before cmd: " + std.loadFile("#task/self/cmd").trim() + "\n");
std.out.puts("before argv: " + scriptArgs.join("|") + "\n");
std.out.puts("before wasi env: " + std.getenv("MODE") + "\n");
std.out.puts("before task env: " + taskMode + "\n");
std.out.flush();
