import * as std from "qjs:std";
import { runtime } from "./qjs-demo-lib.js";

const source = std.loadFile("main.js");

std.writeFile("hello.txt", "hello from a Wanix namespace");

std.out.puts("outside Chrome: " + source.includes("std.loadFile") + "\n");
std.out.puts("task id: " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts(runtime + "\n");
std.out.puts(std.loadFile("hello.txt") + "\n");
std.out.flush();
