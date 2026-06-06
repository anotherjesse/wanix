import * as std from "qjs:std";
import { spawn } from "lib/wanix/task.js";
import { self } from "lib/wanix/process.js";

// Mount a host-provided child script and launch it as a qjs task, using the
// Wanix guest SDK spawn() helper in place of hand-rolled service-file plumbing.

const parent = self.id();
const childSource = std.loadFile("qjs-host-spawn-child.js");
std.writeFile("host/qjs-host-spawn-child.js", childSource);
std.writeFile("host/parent-output.txt", "parent task " + parent);

const child = spawn("qjs", {
  cmd: "host/qjs-host-spawn-child.js mounted 'two words'",
  env: { MODE: "host-child" },
  dir: ".",
  binds: [
    ["#task/" + parent + "/fd/1", 1],
    ["#task/" + parent + "/fd/2", 2],
  ],
});

std.out.puts("parent task: " + parent + "\n");
std.out.puts("child task: " + child.id + "\n");
std.out.flush();
child.start();
std.out.puts("child exit: " + child.wait() + "\n");
std.out.puts("host child output: " + std.loadFile("host/child-output.txt") + "\n");
std.out.flush();
