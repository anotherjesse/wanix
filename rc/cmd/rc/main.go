package main

import (
	"bufio"
	"os"

	"tractor.dev/wanix/rc/shell"
)

func main() {
	// The wanix gojs stdout sink loses single-byte writes when many small
	// writes hit it in quick succession. Buffer here so echo/printf-style
	// per-byte output coalesces into one write per flush.
	stdout := bufio.NewWriter(os.Stdout)
	stderr := bufio.NewWriter(os.Stderr)
	code := shell.Main(os.Args[1:], os.Stdin, stdout, stderr)
	_ = stdout.Flush()
	_ = stderr.Flush()
	os.Exit(code)
}
