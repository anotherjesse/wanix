import * as std from "qjs:std";

const id = std.loadFile("#task/self/id").trim();
const output =
  "child task " + id
  + " args " + scriptArgs.join("|")
  + " mode " + std.getenv("MODE");
std.writeFile("host/child-output.txt", output);
std.out.puts(output + "\n");
std.out.flush();
std.exit(4);
