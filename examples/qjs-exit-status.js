import * as std from "qjs:std";

std.out.puts("before std exit\n");
std.out.flush();
std.exit(7);
std.out.puts("after std exit\n");
