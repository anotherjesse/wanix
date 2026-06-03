import * as std from "qjs:std";

const input = std.loadFile("host/input.txt").trim();
const output = "mounted output for " + input;
std.writeFile("host/output.txt", output);
std.out.puts("host input: " + input + "\n");
std.out.puts("host output: " + std.loadFile("host/output.txt") + "\n");
std.out.flush();
