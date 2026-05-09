// wanix-cli runs a wanix terminal program in headless Chrome and bridges
// the in-browser terminal to local stdin/stdout.
//
// Usage:
//
//	wanix-cli                                  # serves repo root, runs examples/repl-rc
//	wanix-cli -page /examples/repl-gojs/ -task '#task/repl'
//	wanix-cli -dir . -listen :7654
//
// The bridge serves the named directory over HTTP with cross-origin isolation
// headers, opens the page in headless Chrome, attaches reader+writer to the
// task's terminal, and watches the task's exit file. When the task records an
// exit code, wanix-cli prints buffered output and exits with the same code.
// Closing local stdin (EOF) closes the page-side writer so the in-browser
// program sees EOF on stdin.
package main

import (
	"context"
	"encoding/base64"
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/chromedp/cdproto/runtime"
	"github.com/chromedp/chromedp"
)

func main() {
	var (
		listen     = flag.String("listen", "127.0.0.1:0", "http listen addr (random port if :0)")
		dir        = flag.String("dir", ".", "directory to serve")
		page       = flag.String("page", "/examples/repl-rc/", "page path to open")
		taskPath   = flag.String("task", "#task/repl", "wanix task base path (term=<task>/term/data, exit=<task>/exit)")
		chromePath = flag.String("chrome", defaultChrome(), "chrome/chromium executable path")
		showHead   = flag.Bool("head", false, "show browser window (debug)")
		verbose    = flag.Bool("v", false, "verbose logging")
		readyTO    = flag.Duration("ready-timeout", 15*time.Second, "timeout waiting for page bridge ready")
		exitGrace  = flag.Duration("exit-grace", 500*time.Millisecond, "drain delay after task exit before this process exits")
	)
	flag.Parse()

	absDir, err := filepath.Abs(*dir)
	if err != nil {
		log.Fatal(err)
	}

	srv, addr, err := startServer(*listen, absDir)
	if err != nil {
		log.Fatal(err)
	}
	defer srv.Close()

	url := fmt.Sprintf("http://%s%s", addr, *page)
	if *verbose {
		log.Println("serving", absDir, "at http://"+addr)
		log.Println("opening", url)
	}

	opts := append(chromedp.DefaultExecAllocatorOptions[:],
		chromedp.ExecPath(*chromePath),
		chromedp.Flag("headless", !*showHead),
		chromedp.Flag("disable-gpu", true),
		chromedp.Flag("no-sandbox", true),
	)
	allocCtx, cancelAlloc := chromedp.NewExecAllocator(context.Background(), opts...)
	defer cancelAlloc()

	var ctxOpts []chromedp.ContextOption
	if *verbose {
		ctxOpts = append(ctxOpts, chromedp.WithLogf(log.Printf))
	}
	ctx, cancel := chromedp.NewContext(allocCtx, ctxOpts...)
	defer cancel()

	// chromedp.Run isn't safe for concurrent calls on the same context; serialize
	// stdin->page writes through this mutex.
	var sendMu sync.Mutex
	ready := make(chan struct{})
	exitCh := make(chan int, 1)
	var readyOnce, exitOnce sync.Once

	chromedp.ListenTarget(ctx, func(ev interface{}) {
		switch e := ev.(type) {
		case *runtime.EventBindingCalled:
			switch e.Name {
			case "wanixOut":
				data, err := base64.StdEncoding.DecodeString(e.Payload)
				if err != nil {
					log.Println("decode:", err)
					return
				}
				os.Stdout.Write(data)
			case "wanixReady":
				readyOnce.Do(func() { close(ready) })
			case "wanixExit":
				code, perr := strconv.Atoi(strings.TrimSpace(e.Payload))
				if perr != nil {
					code = 1
				}
				exitOnce.Do(func() { exitCh <- code })
			case "wanixLog":
				if *verbose {
					log.Println("page:", e.Payload)
				}
			}
		case *runtime.EventConsoleAPICalled:
			if *verbose {
				for _, arg := range e.Args {
					log.Println("console:", string(arg.Value))
				}
			}
		}
	})

	if err := chromedp.Run(ctx,
		runtime.AddBinding("wanixOut"),
		runtime.AddBinding("wanixLog"),
		runtime.AddBinding("wanixReady"),
		runtime.AddBinding("wanixExit"),
		chromedp.Navigate(url),
		chromedp.Evaluate(setupJS(*taskPath), nil),
	); err != nil {
		log.Fatal(err)
	}

	// Wait for setup to finish before pumping stdin so writes don't race.
	select {
	case <-ready:
		if *verbose {
			log.Println("bridge ready")
		}
	case <-time.After(*readyTO):
		log.Fatal("timeout waiting for page bridge to become ready")
	case <-ctx.Done():
		return
	}

	// stdin -> page writer; on EOF, close page-side writer to signal EOF to the task.
	stdinDone := make(chan struct{})
	go func() {
		defer close(stdinDone)
		buf := make([]byte, 4096)
		for {
			n, rerr := os.Stdin.Read(buf)
			if n > 0 {
				payload := base64.StdEncoding.EncodeToString(buf[:n])
				js := fmt.Sprintf("window.wanixSend(%q)", payload)
				sendMu.Lock()
				perr := chromedp.Run(ctx, chromedp.Evaluate(js, nil))
				sendMu.Unlock()
				if perr != nil {
					if *verbose {
						log.Println("send:", perr)
					}
					return
				}
			}
			if rerr != nil {
				if rerr != io.EOF && *verbose {
					log.Println("stdin:", rerr)
				}
				sendMu.Lock()
				_ = chromedp.Run(ctx, chromedp.Evaluate(`window.wanixCloseWriter && window.wanixCloseWriter()`, nil))
				sendMu.Unlock()
				return
			}
		}
	}()

	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGINT, syscall.SIGTERM)

	exitCode := 0
	select {
	case code := <-exitCh:
		exitCode = code
		if *verbose {
			log.Printf("task exited with %d", code)
		}
		// Give any final output time to drain.
		time.Sleep(*exitGrace)
	case <-sig:
		exitCode = 130
	case <-ctx.Done():
		exitCode = 1
	}
	_ = stdinDone
	os.Exit(exitCode)
}

func defaultChrome() string {
	candidates := []string{
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
		"/Applications/Chromium.app/Contents/MacOS/Chromium",
		"/usr/bin/google-chrome",
		"/usr/bin/chromium",
	}
	for _, c := range candidates {
		if _, err := os.Stat(c); err == nil {
			return c
		}
	}
	return ""
}

func startServer(listenAddr, dir string) (*http.Server, string, error) {
	ln, err := net.Listen("tcp", listenAddr)
	if err != nil {
		return nil, "", err
	}
	mux := http.NewServeMux()
	mux.Handle("/", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		// Cross-origin isolation needed for SharedArrayBuffer.
		w.Header().Set("Cross-Origin-Opener-Policy", "same-origin")
		w.Header().Set("Cross-Origin-Embedder-Policy", "require-corp")
		w.Header().Set("Access-Control-Allow-Origin", "*")
		http.FileServer(http.Dir(dir)).ServeHTTP(w, r)
	}))
	srv := &http.Server{
		Handler:           mux,
		ReadHeaderTimeout: 10 * time.Second,
	}
	go srv.Serve(ln)
	return srv, ln.Addr().String(), nil
}

// setupJS attaches to the wanix-system, opens reader+writer on the task's
// terminal, and starts watching the task's exit file. Once an exit code is
// recorded, it is delivered to the host via the wanixExit binding.
func setupJS(taskBase string) string {
	return fmt.Sprintf(`(async () => {
		const sleep = ms => new Promise(r => setTimeout(r, ms));
		let sys = document.querySelector('wanix-system');
		while (!sys) { await sleep(50); sys = document.querySelector('wanix-system'); }
		while (!sys.isReady) await sleep(50);
		await window.wanixLog('system ready');

		const taskBase = %q;
		const dataPath = taskBase + '/term/data';
		const exitPath = taskBase + '/exit';

		await sys.root.waitFor(dataPath);
		const readable = await sys.root.openReadable(dataPath);
		// open an explicit fd for writes so we can close it (signals EOF to the
		// reader). WritableStream.close() doesn't touch the underlying fd.
		const wfd = await sys.root.openFile(dataPath, 1, 0);
		const reader = readable.getReader();

		window.wanixSend = async (b64) => {
			const bin = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
			await sys.root.write(wfd, bin);
		};
		window.wanixCloseWriter = async () => {
			try { await sys.root.close(wfd); } catch (e) {}
		};
		await window.wanixReady('');

		// term reader -> stdout
		(async () => {
			try {
				while (true) {
					const { done, value } = await reader.read();
					if (done) break;
					if (!value) continue;
					let s = '';
					for (let i = 0; i < value.length; i++) s += String.fromCharCode(value[i]);
					await window.wanixOut(btoa(s));
				}
			} catch (e) {
				await window.wanixLog('reader error: ' + e);
			}
		})();

		// poll exit file for non-empty content.
		(async () => {
			try {
				await sys.root.waitFor(exitPath);
				while (true) {
					const txt = (await sys.root.readText(exitPath)).trim();
					if (txt.length > 0) {
						await window.wanixExit(txt);
						return;
					}
					await sleep(150);
				}
			} catch (e) {
				await window.wanixLog('exit watcher: ' + e);
				await window.wanixExit('1');
			}
		})();
	})().catch(e => window.wanixLog('setup error: ' + e));`, taskBase)
}
