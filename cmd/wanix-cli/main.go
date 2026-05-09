// wanix-cli runs a wanix terminal program in headless Chrome and bridges
// the in-browser terminal to local stdin/stdout.
//
// Usage:
//   wanix-cli                       # serves repo root, runs examples/repl-rc
//   wanix-cli -page /examples/repl-gojs/
//   wanix-cli -dir . -listen :7654 -path '#task/repl/term/data'
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
		termPath   = flag.String("path", "#task/repl/term/data", "wanix term file path")
		chromePath = flag.String("chrome", defaultChrome(), "chrome/chromium executable path")
		showHead   = flag.Bool("head", false, "show browser window (debug)")
		verbose    = flag.Bool("v", false, "verbose logging")
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

	// stdin->page serialized through this mutex; chromedp.Run isn't safe for
	// concurrent calls on the same context.
	var sendMu sync.Mutex
	ready := make(chan struct{})
	var readyOnce sync.Once

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
		chromedp.Navigate(url),
		chromedp.Evaluate(setupJS(*termPath), nil),
	); err != nil {
		log.Fatal(err)
	}

	// Wait for setup to finish before pumping stdin so writes don't race.
	select {
	case <-ready:
		if *verbose {
			log.Println("bridge ready")
		}
	case <-time.After(15 * time.Second):
		log.Fatal("timeout waiting for page bridge to become ready")
	case <-ctx.Done():
		return
	}

	// stdin -> page writer
	go func() {
		buf := make([]byte, 4096)
		for {
			n, err := os.Stdin.Read(buf)
			if n > 0 {
				payload := base64.StdEncoding.EncodeToString(buf[:n])
				js := fmt.Sprintf("window.wanixSend(%q)", payload)
				sendMu.Lock()
				rerr := chromedp.Run(ctx, chromedp.Evaluate(js, nil))
				sendMu.Unlock()
				if rerr != nil {
					if *verbose {
						log.Println("send:", rerr)
					}
					return
				}
			}
			if err != nil {
				if err != io.EOF && *verbose {
					log.Println("stdin:", err)
				}
				return
			}
		}
	}()

	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGINT, syscall.SIGTERM)
	select {
	case <-sig:
	case <-ctx.Done():
	}
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

// setupJS waits for the wanix-system to be ready, opens reader+writer on the
// term file, and wires them to the chromedp bindings.
func setupJS(path string) string {
	return fmt.Sprintf(`(async () => {
		const findSystem = () => {
			const sys = document.querySelector('wanix-system');
			return sys || null;
		};
		const sleep = ms => new Promise(r => setTimeout(r, ms));
		let sys = findSystem();
		while (!sys) { await sleep(50); sys = findSystem(); }
		while (!sys.isReady) await sleep(50);
		await window.wanixLog('system ready');

		const path = %q;
		await sys.root.waitFor(path);
		const readable = await sys.root.openReadable(path);
		const writable = await sys.root.openWritable(path);
		const reader = readable.getReader();
		const writer = writable.getWriter();

		window.wanixSend = async (b64) => {
			const bin = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
			await writer.write(bin);
		};
		await window.wanixReady('');

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
	})().catch(e => window.wanixLog('setup error: ' + e));`, path)
}
