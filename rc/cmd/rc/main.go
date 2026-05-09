package main

import (
	"bufio"
	"io"
	"os"
	"sync"

	"tractor.dev/wanix/rc/shell"
)

// lockedWriter serializes Write calls to an underlying writer. Under the wanix
// gojs target, stdout (fd/1) and stderr (fd/2) bind to the same terminal file,
// and concurrent writes can interleave or get dropped. Funneling all output
// through one mutex-guarded writer keeps the byte stream coherent.
type lockedWriter struct {
	mu sync.Mutex
	w  io.Writer
}

func (l *lockedWriter) Write(p []byte) (int, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.w.Write(p)
}

func main() {
	// One underlying sink so stdout and stderr can't race. Buffer it so
	// per-byte writes (echo, prompts) coalesce into one syscall per flush;
	// the wanix gojs sink loses single-byte writes when many hit it quickly.
	sink := &lockedWriter{w: os.Stdout}
	buffered := bufio.NewWriter(sink)
	code := shell.Main(os.Args[1:], os.Stdin, buffered, buffered)
	_ = buffered.Flush()
	os.Exit(code)
}
