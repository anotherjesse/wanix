import * as std from "qjs:std";

const task = std.loadFile("#task/self/id").trim();
std.writeFile("host/persist-after.txt", "host after task " + task);
std.out.puts("resume task: " + task + "\n");
std.out.puts("vm: " + globalThis.persistedMessage + "\n");
std.out.puts("reattached mode: " + Wanix.env("MODE") + "\n");
std.out.puts("host: " + std.loadFile("host/persist-before.txt") + "\n");
std.out.flush();
std.exit(6);
