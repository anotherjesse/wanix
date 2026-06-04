
import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';
import { WanixP9Handle } from '../wanix/p9.js';
//@ts-ignore
import { WanixHandle } from '../wanix/fs.js';

declare const navigator: unknown;

type Config = {
	discoveryUrl?: string;
	qjsShellUrl?: string;
	qjsTask?: boolean;
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
		context.subscriptions.push(vscode.commands.registerCommand('workbench.runQjsTask', async () => {
			try {
				const term = vscode.window.createTerminal({
					name: await qjsTerminalName(),
					pty: await createActiveQjsTaskTerminal(fsys, bridge, config)
				});
				term.show();
				context.subscriptions.push(term);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
	});
	
	console.log('System extension activated');
}

async function qjsTerminalName(): Promise<string> {
	const editor = activeWanixEditor();
	if (!editor) {
		return "qjs";
	}
	return `qjs: ${baseName(editor.document.uri.path)}`;
}

async function createActiveQjsTaskTerminal(fsys: any, bridge: WanixBridge, config: Config) {
	if (!config.ns?.task || !config.ns?.term) {
		throw new Error("Wanix task and terminal services are not available");
	}
	if (config.qjsTask === false) {
		throw new Error("Wanix discovery did not advertise the qjs task driver");
	}
	const editor = activeWanixEditor();
	if (!editor) {
		throw new Error("Open a wanix: JavaScript file before running a qjs task");
	}
	if (editor.document.isDirty && !(await editor.document.save())) {
		throw new Error("Save the active file before running it as a qjs task");
	}

	const scriptPath = bridge.normalizePath(editor.document.uri.path);
	const scriptDir = parentPath(scriptPath) || ".";
	const scriptName = baseName(scriptPath);
	return await createTerminal(fsys, {
		...config,
		qjsShellUrl: undefined,
		shell: {
			cmd: quoteShellArg(scriptName),
			type: "qjs",
			wd: scriptDir
		}
	});
}

function activeWanixEditor(): vscode.TextEditor | undefined {
	const editor = vscode.window.activeTextEditor;
	if (editor?.document.uri.scheme === WanixBridge.scheme) {
		return editor;
	}
	return undefined;
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
	await fsys.writeFile(`${taskPath}/ctl`, `bind ${quoteShellArg(`${termPath}/program`)} fd/0`);
	await fsys.writeFile(`${taskPath}/ctl`, `bind ${quoteShellArg(`${termPath}/program`)} fd/1`);
	await fsys.writeFile(`${taskPath}/ctl`, `bind ${quoteShellArg(`${termPath}/program`)} fd/2`);
	await fsys.writeFile(`${taskPath}/ctl`, "start");

	const writeEmitter = new vscode.EventEmitter<string>();
	const dec = new TextDecoder();
	const enc = new TextEncoder();
	const readable = await fsys.openReadable(`${termPath}/data`);
	const writable = (await fsys.openWritable(`${termPath}/data`)).getWriter();
	let pendingResize: Promise<void> = Promise.resolve();
	let closed = false;
	let buffer = '';
	const sendResize = (dimensions: vscode.TerminalDimensions) => {
		if (dimensions.columns <= 0 || dimensions.rows <= 0 || closed) {
			return;
		}
		const payload = enc.encode(`${dimensions.columns} ${dimensions.rows}\n`);
		pendingResize = pendingResize.then(async () => {
			const winch = (await fsys.openWritable(`${termPath}/winch`)).getWriter();
			try {
				await winch.write(payload);
			} finally {
				await winch.close();
			}
		}).catch((error) => {
			console.warn("Wanix terminal resize failed", error);
		});
	};
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
			closed = true;
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
		setDimensions: (dimensions: vscode.TerminalDimensions) => {
			sendResize(dimensions);
		}
	};
}

function splitPath(path: string): string[] {
	return path.split("/").filter(Boolean);
}

function parentPath(path: string): string {
	const parts = splitPath(path);
	parts.pop();
	return parts.join("/");
}

function baseName(path: string): string {
	const parts = splitPath(path);
	return parts.pop() || path;
}

function quoteShellArg(arg: string): string {
	if (arg.length > 0 && !/[\s'"\\]/.test(arg)) {
		return arg;
	}
	return `'${arg.replace(/'/g, `'\"'\"'`)}'`;
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
