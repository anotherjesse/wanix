import * as std from "qjs:std";
import * as os from "qjs:os";

const result = os.mkdir("generated", 0o777);
std.writeFile("generated/message.txt", "hello from a created directory");

std.out.puts("mkdir: " + result + "\n");
std.out.puts("message: " + std.loadFile("generated/message.txt") + "\n");
