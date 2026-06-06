import * as std from "qjs:std";
import { spawn } from "lib/wanix/task.js";
import { self } from "lib/wanix/process.js";

// Launch a child qjs task through the #task service files using the Wanix guest
// SDK instead of hand-rolled readServiceText/writeServiceText helpers.

const parent = self.id();
std.writeFile("child stdin.txt", "stdin from parent\n");

const child = spawn("qjs", {
  cmd: "qjs-task-spawn-child.js alpha 'two words' '' beta",
  env: { MODE: "spawned" },
  dir: ".",
  binds: [
    ["child stdin.txt", 0],
    ["#task/" + parent + "/fd/1", 1],
    ["#task/" + parent + "/fd/2", 2],
  ],
});

print("parent task: " + parent);
print("child task: " + child.id);
child.start();
print("child exit: " + child.wait());
