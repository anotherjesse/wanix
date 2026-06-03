const input = Wanix.readFd(0, 1024);

print("stdin:", input.trimEnd());
print("task id:", Wanix.readText("#task/self/id").trim());
