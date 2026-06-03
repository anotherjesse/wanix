import * as std from "qjs:std";
import * as os from "qjs:os";

os.mkdir("gone", 0o777);
os.mkdir("kept", 0o777);
std.writeFile("kept/message.txt", "still here");

const result = os.remove("gone");
const probe = os.open("gone", os.O_RDONLY);
if (probe >= 0) {
  os.close(probe);
}

std.out.puts("remove dir: " + result + "\n");
std.out.puts("gone: " + (probe < 0) + "\n");
std.out.puts("kept: " + std.loadFile("kept/message.txt") + "\n");
