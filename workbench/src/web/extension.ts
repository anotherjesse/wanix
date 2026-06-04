
import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';
import { WanixP9Handle } from '../wanix/p9.js';
//@ts-ignore
import { WanixHandle } from '../wanix/fs.js';

declare const navigator: unknown;

type Config = {
	discoveryUrl?: string;
	qjsShellUrl?: string;
	term?: boolean;
	raw?: boolean;
	ns?: {
		task: string;
		term: string;
	}
	shell?: {
		cmd: string;
		type: string;
		wd: string;
	}
}

export async function activate(context: vscode.ExtensionContext) {
	if (typeof navigator !== 'object') {	// do not run under node.js
		console.error("not running in browser");
		return;
	}
	
	let config: Config = {};
	const wanix = createWanixHandle(context, (nextConfig) => {
		config = nextConfig;
	});
	const bridge = new WanixBridge(wanix, "");
	context.subscriptions.push(bridge);

	bridge.ready.then((fsys) => {
		fsys.logger = (...args: any[]) => {
			// console.log(...args);
		};
		if (config.qjsShellUrl || config.shell) {
			context.subscriptions.push(vscode.commands.registerCommand('workbench.createTerminal', async () => {
				const term = vscode.window.createTerminal({ 
					name: 'Shell', 
					pty: config.qjsShellUrl ? createQjsShellTerminal(config) : await createTerminal(fsys, config)
				});
				term.show();
				context.subscriptions.push(term);
			}));
			if (config.term) {
				vscode.commands.executeCommand(`workbench.createTerminal`);
			}
			
		}
	});
	
	console.log('System extension activated');
}

function createWanixHandle(context: vscode.ExtensionContext, setConfig: (config: Config) => void): Promise<any> {
	const channel = new MessageChannel();
	return new Promise<any>((resolve) => {
		let settled = false;
		let pendingConfig: Config = {
			discoveryUrl: new URL("/.well-known/wanix.json", context.extensionUri.toString()).href,
		};
		const resolveHandle = (handle: any, nextConfig: Config) => {
			if (settled) {
				return;
			}
			settled = true;
			setConfig(nextConfig);
			resolve(handle);
		};

		channel.port2.onmessage = async (event) => {
			if (event.data.config) {
				pendingConfig = { ...pendingConfig, ...event.data.config };
			}
			if (event.data.wanix) {
				resolveHandle(new WanixHandle(event.data.wanix), pendingConfig);
			}
		};

		const port = (context as any).messagePassingProtocol;
		if (port?.postMessage) {
			port.postMessage({type: "_port", port: channel.port1}, [channel.port1]);
		}

		delay(100).then(() => {
			if (settled) {
				return undefined;
			}
			return WanixP9Handle.fromDiscovery(pendingConfig.discoveryUrl);
		}).then((handle) => {
			if (handle) {
				resolveHandle(handle, pendingConfig);
			}
		}).catch((error) => {
			if (!settled) {
				console.warn("Wanix direct 9P discovery fallback failed", error);
			}
		});
	});
}

function delay(ms: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, ms));
}

function createQjsShellTerminal(config: Config) {
	const writeEmitter = new vscode.EventEmitter<string>();
	const dec = new TextDecoder();
	const enc = new TextEncoder();
	let socket: WebSocket | undefined;
	let opened = false;
	const pending: Uint8Array[] = [];
	const sendInput = (bytes: Uint8Array) => {
		if (opened && socket?.readyState === WebSocket.OPEN) {
			socket.send(bytes);
		} else {
			pending.push(bytes);
		}
	};
	return {
		onDidWrite: writeEmitter.event,
		open: () => {
			socket = new WebSocket(config.qjsShellUrl || "");
			socket.binaryType = "arraybuffer";
			socket.onopen = () => {
				opened = true;
				while (pending.length > 0) {
					socket?.send(pending.shift()!);
				}
			};
			socket.onmessage = async (event) => {
				if (typeof event.data === "string") {
					try {
						const message = JSON.parse(event.data);
						if (message.type === "error") {
							writeEmitter.fire(`\r\n${message.message}\r\n`);
						}
					} catch {
						// Ignore lifecycle text frames that are not terminal output.
					}
					return;
				}
				const bytes = event.data instanceof Blob
					? await event.data.arrayBuffer()
					: event.data;
				writeEmitter.fire(dec.decode(bytes));
			};
			socket.onerror = () => {
				writeEmitter.fire("\r\nterminal websocket failed\r\n");
			};
		},
		close: () => {
			socket?.close();
		},
		handleInput: (data: string) => {
			sendInput(enc.encode(data));
		},
		setDimensions: (dimensions: vscode.TerminalDimensions) => {
			if (socket?.readyState === WebSocket.OPEN) {
				socket.send(JSON.stringify({
					type: "resize",
					columns: dimensions.columns,
					rows: dimensions.rows
				}));
			}
		}
	};
}

async function createTerminal(fsys: any, config: Config) {
	await fsys.waitFor(config.ns?.task, 30000);
	const termID = (await fsys.readText(`${config.ns?.term}/new`)).trim();
    const termPath = [config.ns?.term, termID].join("/");
	const taskID = (await fsys.readText(`${config.ns?.task}/new/${config.shell?.type || 'auto'}`)).trim();
	const taskPath = [config.ns?.task, taskID].join("/");
	await fsys.writeFile(`${taskPath}/cmd`, config.shell?.cmd);
	await fsys.writeFile(`${taskPath}/dir`, config.shell?.wd);
	// not sure the best way to do this but the bind paths need to be 
	// relative to the root of that system. works fine until you change 
	// namespaces to a mount of another system, because the bind paths need
	// to be relative to the root of the new system. this is a hack for now:
	const commonPath = (a: string, b: string, sep = '/') => {
		const as = a.split(sep);
		const bs = b.split(sep);
		const out = [];
		for (let i = 0; i < Math.min(as.length, bs.length); i++) {
		  if (as[i] !== bs[i]) break;
		  out.push(as[i]);
		}
		return out.join(sep);
	}
	const common = commonPath(termPath, taskPath);
	const termPathInner = termPath.slice(common.length+1);
	const taskPathInner = taskPath.slice(common.length+1);
	// console.log(`bind ${termPathInner}/program ${taskPathInner}/fd/0`);
	await fsys.writeFile(`${taskPath}/ctl`, `bind ${termPathInner}/program ${taskPathInner}/fd/0`);
	await fsys.writeFile(`${taskPath}/ctl`, `bind ${termPathInner}/program ${taskPathInner}/fd/1`);
	await fsys.writeFile(`${taskPath}/ctl`, `bind ${termPathInner}/program ${taskPathInner}/fd/2`);
	await fsys.writeFile(`${taskPath}/ctl`, "start");

	const writeEmitter = new vscode.EventEmitter<string>();
	const dec = new TextDecoder();
	const enc = new TextEncoder();
	const readable = await fsys.openReadable(`${termPath}/data`);
	const writable = (await fsys.openWritable(`${termPath}/data`)).getWriter();
	let buffer = '';
	return {
		onDidWrite: writeEmitter.event,
		open: () => {
			(async () => {
				for await (const chunk of readable) {
					writeEmitter.fire(dec.decode(chunk));
				}
			})();
		},
		close: () => {
			writable.close();
		},
		handleInput: async (data: string) => {
			if (config.raw) {
				writable.write(enc.encode(data));
				return;
			}
			// may add line discipline as mode to terminals but for now we
			// do as plan 9 and handle it here in "userspace"
			if (data === '\r') {
				writeEmitter.fire('\r\n');           // echo newline
				writable.write(enc.encode(buffer+"\n"));
				buffer = '';
			} else if (data === '\x7f') {   // backspace
				if (buffer.length > 0) {
					buffer = buffer.slice(0, -1);
					writeEmitter.fire('\b \b');
				}
			} else {
				buffer += data;
				writeEmitter.fire(data);             // echo
			}
		},
		setDimensions: async (dimensions: vscode.TerminalDimensions) => {
			// const winch = (await fsys.openWritable(`${termPath}/winch`)).getWriter();
			// await winch.write(enc.encode(`${dimensions.columns} ${dimensions.rows}\n`));
			// await winch.close();
		}
	};
}

// @ts-ignore
// polyfill for ReadableStream.prototype[Symbol.asyncIterator] on safari
if (!ReadableStream.prototype[Symbol.asyncIterator]) {
	// @ts-ignore
    ReadableStream.prototype[Symbol.asyncIterator] = async function* () {
        const reader = this.getReader();
        try {
            while (true) {
                const { done, value } = await reader.read();
                if (done) return;
                yield value;
            }
        } finally {
            reader.releaseLock();
        }
    };
}
