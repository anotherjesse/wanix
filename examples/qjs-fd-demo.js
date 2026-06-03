const input = Wanix.open("main.js", "r");
print("read fd:", input);
print("saw fd API:", Wanix.readFd(input, 80).includes("Wanix.open"));
Wanix.closeFd(input);

const output = Wanix.open("fd-output.txt", "w+");
print("write fd:", output);
print("bytes:", Wanix.writeFd(output, "hello from a Wanix fd"));
Wanix.closeFd(output);
print(Wanix.readText("fd-output.txt"));
