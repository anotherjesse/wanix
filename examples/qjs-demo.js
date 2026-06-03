import { runtime } from "./qjs-demo-lib.js";

const source = Wanix.readText("main.js");

Wanix.writeText("hello.txt", "hello from a Wanix namespace");

print("outside Chrome:", source.includes("Wanix.readText"));
print("task id:", Wanix.readText("#task/self/id").trim());
print(runtime);
print(Wanix.readText("hello.txt"));
