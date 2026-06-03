print("task id:", Wanix.readText("#task/self/id").trim());
print("cwd:", Wanix.cwd());
print("args:", Wanix.args().join(","));
print("mode:", Wanix.env("MODE") ?? "(unset)");
