import * as std from "qjs:std";

std.out.puts("task id: " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts("mode: " + (std.getenv("MODE") ?? "(unset)") + "\n");
std.out.puts("args: " + scriptArgs.slice(1).join(",") + "\n");
std.out.puts("source in cwd: " + std.loadFile("main.js").includes("qjs:std") + "\n");
std.out.flush();
