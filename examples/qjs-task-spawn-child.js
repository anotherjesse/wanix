import * as std from "qjs:std";

std.writeFile(
  "child-result.txt",
  "child task "
    + std.loadFile("#task/self/id").trim()
    + " args "
    + scriptArgs.join("|")
    + " mode "
    + std.getenv("MODE")
);
std.exit(5);
