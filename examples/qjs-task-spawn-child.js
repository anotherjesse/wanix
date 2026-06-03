import * as std from "qjs:std";

std.out.puts(
  "child task "
    + std.loadFile("#task/self/id").trim()
    + " args "
    + scriptArgs.join("|")
    + " mode "
    + std.getenv("MODE")
    + "\n"
);
std.out.flush();
std.err.puts("child stderr mode " + std.getenv("MODE") + "\n");
std.err.flush();
std.exit(5);
