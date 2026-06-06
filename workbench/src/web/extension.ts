
import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';
import { DUET_DEMO_STEPS, DUET_OUTPUT_PATH, installDuetDemo } from './duet-demo.js';
import { copyHttpAppUrl, installHttpAppDemo, openHttpAppDemo, openHttpAppHandler, type HttpAppRouteConfig } from './http-app-demo.js';
import { createQjsStarter } from './qjs-starter.js';
import { WanixSystemView } from './system-view.js';
import { WanixP9Handle, type WanixP9Route } from '../wanix/p9.js';
//@ts-ignore
import { WanixHandle } from '../wanix/fs.js';

declare const navigator: unknown;

type Config = {
	discoveryUrl?: string;
	p9?: WanixP9Route;
	qjsShellUrl?: string;
	qjsTask?: boolean;
	drivers?: string[];
	httpApp?: HttpAppRouteConfig;
	term?: boolean;
	raw?: boolean;
	open?: string;
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

type TaskRunKind = "qjs" | "wasm";

type TaskRunTarget = {
	path: string;
	dir: string;
	name: string;
};

const TASK_RUNNERS: Record<TaskRunKind, { extension: string; label: string }> = {
	qjs: { extension: ".js", label: "JavaScript" },
	wasm: { extension: ".wasm", label: "WASM" },
};
const TASK_OUTPUT_DIR = ".wanix/tasks";
const TASK_OUTPUT_MAX_CHARS = 512 * 1024;

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
	const systemView = new WanixSystemView();
	systemView.register(context);
	const refreshFiles = async () => {
		await refreshWorkbenchFiles(bridge);
		systemView.filesystemActivity("filesystem refreshed");
	};
	const activeTaskTerminals = new Map<TaskRunKind, vscode.Terminal>();
	const taskTerminals = new Map<string, vscode.Terminal>();
	context.subscriptions.push(bridge);
	rememberWanixEditor();
	context.subscriptions.push(vscode.window.onDidChangeActiveTextEditor((editor) => {
		rememberWanixEditor(editor);
	}));
	context.subscriptions.push(vscode.window.onDidCloseTerminal((terminal) => {
		for (const [kind, activeTerminal] of activeTaskTerminals) {
			if (terminal === activeTerminal) {
				activeTaskTerminals.delete(kind);
			}
		}
		for (const [taskId, taskTerminal] of taskTerminals) {
			if (terminal === taskTerminal) {
				taskTerminals.delete(taskId);
			}
		}
	}));

	bridge.ready.then((fsys) => {
		systemView.configure(config);
		revealWanixSystemView();
		fsys.logger = (...args: any[]) => {
			// console.log(...args);
		};
		if (config.qjsShellUrl || config.shell) {
			context.subscriptions.push(vscode.commands.registerCommand('workbench.createTerminal', async () => {
				if (config.qjsShellUrl) {
					systemView.terminalOpened("shell", "Shell");
				}
				const term = vscode.window.createTerminal({ 
					name: 'Shell', 
					pty: config.qjsShellUrl
						? createQjsShellTerminal(config, refreshFiles, systemView)
						: await createTerminal(fsys, config, refreshFiles, {
							taskLabel: "shell",
							terminalLabel: "Shell",
							onTaskStarted: (event) => systemView.taskStarted(event.id, event.kind, event.label),
							onTaskExited: (event) => systemView.taskExited(event.id, event.code),
							onTaskClosed: (event) => systemView.taskClosed(event.id),
							onTerminalOpened: (event) => systemView.terminalOpened(event.id, event.label),
							onTerminalClosed: (event) => systemView.terminalClosed(event.id),
						})
				});
				term.show();
				context.subscriptions.push(term);
			}));
			if (config.term) {
				vscode.commands.executeCommand(`workbench.createTerminal`);
			}
			
		}
		openConfiguredDocument(config).catch((error: unknown) => {
			vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
		});
		context.subscriptions.push(vscode.commands.registerCommand('workbench.runQjsTask', (resource?: vscode.Uri) => {
			runWanixTask(fsys, bridge, config, systemView, activeTaskTerminals, taskTerminals, "qjs", resource, context);
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.runWasmTask', (resource?: vscode.Uri) => {
			runWanixTask(fsys, bridge, config, systemView, activeTaskTerminals, taskTerminals, "wasm", resource, context);
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.installDuetDemo', async () => {
			try {
				await installDuetDemo(context, fsys, bridge, systemView);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.runDuetDemo', async () => {
			try {
				await runDuetDemo(fsys, bridge, config, systemView, activeTaskTerminals, taskTerminals, context);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.newQjsScript', async () => {
			try {
				await createQjsStarter(fsys, bridge, systemView);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openWanixTaskSource', async (source?: string | { sourcePath?: string }) => {
			try {
				const sourcePath = taskSourcePath(source);
				if (!sourcePath) {
					throw new Error("Task source is not available");
				}
				await openWanixPath(sourcePath);
				systemView.filesystemActivity(`opened task source ${baseName(sourcePath)}`);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openWanixTaskOutput', async (output?: string | { outputPath?: string }) => {
			try {
				const outputPath = taskOutputPath(output);
				if (!outputPath) {
					throw new Error("Task output is not available");
				}
				await openWanixPath(outputPath);
				systemView.filesystemActivity(`opened task output ${baseName(outputPath)}`);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.focusTaskTerminal', async (task?: string | { taskId?: string }) => {
			try {
				const taskId = taskIdFromArgument(task);
				if (!taskId) {
					throw new Error("Task terminal is not available");
				}
				const terminal = taskTerminals.get(taskId);
				if (!terminal) {
					throw new Error(`Task ${taskId} terminal is no longer open`);
				}
				terminal.show();
				systemView.filesystemActivity(`focused task terminal ${taskId}`);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.installHttpAppDemo', async () => {
			try {
				await installHttpAppDemo(fsys, bridge, systemView);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openHttpAppDemo', async () => {
			try {
				await openHttpAppDemo(fsys, bridge, config, systemView);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openHttpAppHandler', async () => {
			try {
				await openHttpAppHandler(fsys, bridge, systemView);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.copyHttpAppUrl', async () => {
			try {
				await copyHttpAppUrl(config, systemView);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
	});
	
	console.log('System extension activated');
}

async function openConfiguredDocument(config: Config): Promise<void> {
	const uri = configuredOpenUri(config.open);
	if (!uri) {
		return;
	}
	await openWanixUri(uri);
}

async function openWanixUri(uri: vscode.Uri): Promise<void> {
	const document = await vscode.workspace.openTextDocument(uri);
	await vscode.window.showTextDocument(document, { preview: false });
	rememberWanixEditor();
}

async function openWanixPath(path: string): Promise<void> {
	await openWanixUri(vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path: absoluteWanixPath(path),
	}));
}

function absoluteWanixPath(path: string): string {
	const trimmed = path.trim();
	return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
}

function taskSourcePath(source: string | { sourcePath?: string } | undefined): string | undefined {
	if (typeof source === "string") {
		return source;
	}
	return source?.sourcePath;
}

function taskOutputPath(output: string | { outputPath?: string } | undefined): string | undefined {
	if (typeof output === "string") {
		return output;
	}
	return output?.outputPath;
}

function taskIdFromArgument(task: string | { taskId?: string } | undefined): string | undefined {
	if (typeof task === "string") {
		return task;
	}
	return task?.taskId;
}

function configuredOpenUri(target: string | undefined): vscode.Uri | undefined {
	const trimmed = target?.trim();
	if (!trimmed) {
		return undefined;
	}
	if (/^[a-zA-Z][\w+.-]*:/.test(trimmed)) {
		const uri = vscode.Uri.parse(trimmed);
		if (uri.scheme !== WanixBridge.scheme) {
			throw new Error("Wanix workbench open= only supports wanix: paths");
		}
		return uri;
	}
	const path = trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
	return vscode.Uri.from({ scheme: WanixBridge.scheme, path });
}

async function runWanixTask(
	fsys: any,
	bridge: WanixBridge,
	config: Config,
	systemView: WanixSystemView,
	activeTaskTerminals: Map<TaskRunKind, vscode.Terminal>,
	taskTerminals: Map<string, vscode.Terminal>,
	kind: TaskRunKind,
	resource: vscode.Uri | undefined,
	context: vscode.ExtensionContext,
): Promise<void> {
	try {
		const target = await taskRunTarget(kind, bridge, resource);
		await runWanixTaskTarget(fsys, bridge, config, systemView, activeTaskTerminals, taskTerminals, kind, target, context);
	} catch (error) {
		vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
	}
}

async function runDuetDemo(
	fsys: any,
	bridge: WanixBridge,
	config: Config,
	systemView: WanixSystemView,
	activeTaskTerminals: Map<TaskRunKind, vscode.Terminal>,
	taskTerminals: Map<string, vscode.Terminal>,
	context: vscode.ExtensionContext,
): Promise<void> {
	await installDuetDemo(context, fsys, bridge, systemView, { openProducer: false, notify: false });
	systemView.filesystemActivity("duet demo run started");
	for (const step of DUET_DEMO_STEPS) {
		const code = await runWanixTaskTarget(
			fsys,
			bridge,
			config,
			systemView,
			activeTaskTerminals,
			taskTerminals,
			step.kind,
			taskRunTargetFromPath(bridge, step.kind, step.path),
			context,
			{ waitForExit: true },
		);
		if (code !== 0) {
			throw new Error(`Duet ${step.label} exited ${formatTaskExitCode(code)}`);
		}
	}
	systemView.filesystemActivity("duet demo verified");
	await openWanixPath(DUET_OUTPUT_PATH);
	vscode.window.showInformationMessage("Wanix JS and WASM duet demo completed");
}

async function runWanixTaskTarget(
	fsys: any,
	bridge: WanixBridge,
	config: Config,
	systemView: WanixSystemView,
	activeTaskTerminals: Map<TaskRunKind, vscode.Terminal>,
	taskTerminals: Map<string, vscode.Terminal>,
	kind: TaskRunKind,
	target: TaskRunTarget,
	context: vscode.ExtensionContext,
	options: { waitForExit?: boolean } = {},
): Promise<number | undefined> {
	activeTaskTerminals.get(kind)?.dispose();
	activeTaskTerminals.delete(kind);
	let resolveExit: (code: number | undefined) => void = () => {};
	const exit = new Promise<number | undefined>((resolve) => {
		resolveExit = resolve;
	});
	let startedTaskId: string | undefined;
	const pty = await createActiveTaskTerminal(fsys, bridge, config, systemView, kind, target, {
		onStart: (taskId) => {
			startedTaskId = taskId;
		},
		onExit: resolveExit,
		onClose: () => {
			if (startedTaskId) {
				taskTerminals.delete(startedTaskId);
			}
			resolveExit(undefined);
		},
	});
	const term = vscode.window.createTerminal({
		name: taskTerminalName(kind, target.name),
		pty,
	});
	activeTaskTerminals.set(kind, term);
	if (startedTaskId) {
		taskTerminals.set(startedTaskId, term);
	}
	term.show();
	context.subscriptions.push(term);
	return options.waitForExit ? await exit : undefined;
}

async function createActiveTaskTerminal(
	fsys: any,
	bridge: WanixBridge,
	config: Config,
	systemView: WanixSystemView,
	kind: TaskRunKind,
	target: TaskRunTarget,
	lifecycle: { onStart?: (taskId: string) => void; onExit?: (code: number | undefined) => void; onClose?: () => void } = {},
) {
	if (!config.ns?.task || !config.ns?.term) {
		throw new Error("Wanix task and terminal services are not available");
	}
	if (!taskDriverAdvertised(config, kind)) {
		throw new Error(`Wanix discovery did not advertise the ${kind} task driver`);
	}
	const output = new TaskOutputRecorder(fsys, bridge, systemView, kind, target);
	return await createTerminal(fsys, {
		...config,
		qjsShellUrl: undefined,
		shell: {
			cmd: quoteShellArg(target.name),
			type: kind,
			wd: target.dir
		}
	}, async () => {
		await refreshWorkbenchFiles(bridge);
		systemView.filesystemActivity(`${kind} task filesystem refresh`);
		revealWanixSystemView();
	}, {
		keepOpenOnExit: true,
		taskLabel: target.name,
		terminalLabel: taskTerminalName(kind, target.name),
		onTaskStarted: (event) => {
			lifecycle.onStart?.(event.id);
			const outputPath = output.start(event.id);
			systemView.taskStarted(event.id, event.kind, event.label, { sourcePath: target.path, outputPath });
			revealWanixSystemView();
		},
		onTaskExited: (event) => {
			systemView.taskExited(event.id, event.code);
			delay(0).then(() => output.finish(event.code)).catch((error) => {
				console.warn("Wanix task output write failed", error);
			});
			lifecycle.onExit?.(event.code);
			revealWanixSystemView();
		},
		onTaskClosed: (event) => {
			systemView.taskClosed(event.id);
			delay(0).then(() => output.finish()).catch((error) => {
				console.warn("Wanix task output write failed", error);
			});
			lifecycle.onClose?.();
		},
		onTaskOutput: (event) => output.append(event.chunk),
		onTerminalOpened: (event) => systemView.terminalOpened(event.id, event.label),
		onTerminalClosed: (event) => systemView.terminalClosed(event.id),
	});
}

class TaskOutputRecorder {
	private chunks: string[] = [];
	private totalChars = 0;
	private truncated = false;
	private saved = false;
	private outputPath: string | undefined;

	constructor(
		private readonly fsys: any,
		private readonly bridge: WanixBridge,
		private readonly systemView: WanixSystemView,
		private readonly kind: TaskRunKind,
		private readonly target: TaskRunTarget,
	) {}

	start(taskId: string): string {
		this.outputPath = `${TASK_OUTPUT_DIR}/${taskId}-${this.kind}-${safeOutputFileName(this.target.name)}.output.txt`;
		return this.outputPath;
	}

	append(chunk: string): void {
		if (this.saved || chunk.length === 0) {
			return;
		}
		const remaining = TASK_OUTPUT_MAX_CHARS - this.totalChars;
		if (remaining <= 0) {
			this.truncated = true;
			return;
		}
		const next = chunk.length > remaining ? chunk.slice(0, remaining) : chunk;
		this.chunks.push(next);
		this.totalChars += next.length;
		if (next.length < chunk.length) {
			this.truncated = true;
		}
	}

	async finish(exitCode?: number): Promise<void> {
		if (this.saved || !this.outputPath) {
			return;
		}
		this.saved = true;
		await this.fsys.makeDirAll(TASK_OUTPUT_DIR);
		await this.fsys.writeFile(this.outputPath, this.render(exitCode));
		this.bridge.refresh(`/${TASK_OUTPUT_DIR}`);
		this.bridge.refresh(`/${this.outputPath}`);
		this.systemView.filesystemActivity(`wrote task output ${baseName(this.outputPath)}`);
		await refreshWorkbenchFiles(this.bridge);
	}

	private render(exitCode?: number): string {
		const exit = typeof exitCode === "number" ? String(exitCode) : "?";
		const truncation = this.truncated
			? `\n[wanix transcript truncated at ${TASK_OUTPUT_MAX_CHARS} chars]\n`
			: "";
		return [
			`task: ${this.kind} ${this.target.name}`,
			`source: ${absoluteWanixPath(this.target.path)}`,
			`exit: ${exit}`,
			"",
			"--- output ---",
			this.chunks.join(""),
			truncation,
		].join("\n");
	}
}

async function taskRunTarget(kind: TaskRunKind, bridge: WanixBridge, resource?: vscode.Uri): Promise<TaskRunTarget> {
	const uri = commandWanixUri(resource) || currentWanixEditor()?.document.uri;
	if (!uri) {
		throw new Error(`Open or select a wanix: ${TASK_RUNNERS[kind].label} file before running a ${kind} task`);
	}
	if (uri.scheme !== WanixBridge.scheme) {
		throw new Error(`Run ${kind} expects a wanix: file`);
	}
	const path = bridge.normalizePath(uri.path);
	const expectedExtension = TASK_RUNNERS[kind].extension;
	if (!path.endsWith(expectedExtension)) {
		throw new Error(`Run ${kind} expects a ${expectedExtension} file`);
	}
	await saveOpenDocument(uri, kind);
	return {
		path,
		dir: parentPath(path) || ".",
		name: baseName(path),
	};
}

function taskRunTargetFromPath(bridge: WanixBridge, kind: TaskRunKind, path: string): TaskRunTarget {
	const normalized = bridge.normalizePath(path);
	const expectedExtension = TASK_RUNNERS[kind].extension;
	if (!normalized.endsWith(expectedExtension)) {
		throw new Error(`Run ${kind} expects a ${expectedExtension} file`);
	}
	return {
		path: normalized,
		dir: parentPath(normalized) || ".",
		name: baseName(normalized),
	};
}

function commandWanixUri(resource: vscode.Uri | undefined): vscode.Uri | undefined {
	if (resource instanceof vscode.Uri) {
		return resource;
	}
	return undefined;
}

async function saveOpenDocument(uri: vscode.Uri, kind: TaskRunKind): Promise<void> {
	const document = vscode.workspace.textDocuments.find((candidate) => candidate.uri.toString() === uri.toString());
	if (document?.isDirty && !(await document.save())) {
		throw new Error(`Save ${baseName(uri.path)} before running it as a ${kind} task`);
	}
}

function taskDriverAdvertised(config: Config, kind: TaskRunKind): boolean {
	if (config.drivers) {
		return config.drivers.includes(kind);
	}
	if (kind === "qjs") {
		return config.qjsTask !== false;
	}
	return false;
}

function taskTerminalName(kind: TaskRunKind, name: string): string {
	return `${kind}: ${name}`;
}

function formatTaskExitCode(code: number | undefined): string {
	return typeof code === "number" ? String(code) : "before reporting an exit code";
}

async function refreshWorkbenchFiles(bridge: WanixBridge): Promise<void> {
	bridge.refresh();
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
}

function revealWanixSystemView(): void {
	Promise.resolve(vscode.commands.executeCommand("workbench.view.extension.wanix")).catch((error: unknown) => {
		console.warn("Wanix system view focus failed", error);
	});
}

function activeWanixEditor(): vscode.TextEditor | undefined {
	const editor = vscode.window.activeTextEditor;
	return isWanixEditor(editor) ? editor : undefined;
}

let lastWanixEditor: vscode.TextEditor | undefined;

function currentWanixEditor(): vscode.TextEditor | undefined {
	return activeWanixEditor() || lastWanixEditor;
}

function rememberWanixEditor(editor = vscode.window.activeTextEditor): void {
	if (isWanixEditor(editor)) {
		lastWanixEditor = editor;
	}
}

function isWanixEditor(editor: vscode.TextEditor | undefined): editor is vscode.TextEditor {
	if (editor?.document.uri.scheme === WanixBridge.scheme) {
		return true;
	}
	return false;
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
			if (pendingConfig.p9?.websocket) {
				return WanixP9Handle.fromRoute(pendingConfig.p9);
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

const DIRECT_TASK_EXIT_POLL_MS = 100;
const DIRECT_TASK_EXIT_DRAIN_MS = 100;

function createQjsShellTerminal(config: Config, onFilesystemActivity?: () => void, systemView?: WanixSystemView) {
	const writeEmitter = new vscode.EventEmitter<string>();
	const closeEmitter = new vscode.EventEmitter<number | void>();
	const dec = new TextDecoder();
	const enc = new TextEncoder();
	let socket: WebSocket | undefined;
	let opened = false;
	let closed = false;
	let shellTaskId: string | undefined;
	const pending: Uint8Array[] = [];
	let pendingResize: vscode.TerminalDimensions | undefined;
	const qjsShellUrl = () => {
		const url = new URL(config.qjsShellUrl || "");
		const cwd = config.shell?.wd;
		if (cwd && cwd !== ".") {
			url.searchParams.set("cwd", cwd);
		}
		return url.toString();
	};
	const finish = (code?: number) => {
		if (closed) {
			return;
		}
		closed = true;
		if (shellTaskId) {
			if (typeof code === "number") {
				systemView?.taskExited(shellTaskId, code);
			} else {
				systemView?.taskClosed(shellTaskId);
			}
		}
		systemView?.terminalClosed("shell");
		closeEmitter.fire(code);
	};
	const sendInput = (bytes: Uint8Array) => {
		if (closed) {
			return;
		}
		if (opened && socket?.readyState === WebSocket.OPEN) {
			socket.send(bytes);
		} else {
			pending.push(bytes);
		}
	};
	const sendResize = (dimensions: vscode.TerminalDimensions) => {
		if (dimensions.columns <= 0 || dimensions.rows <= 0 || closed) {
			return;
		}
		const payload = JSON.stringify({
			type: "resize",
			columns: dimensions.columns,
			rows: dimensions.rows
		});
		if (opened && socket?.readyState === WebSocket.OPEN) {
			socket.send(payload);
		} else {
			pendingResize = dimensions;
		}
	};
	return {
		onDidWrite: writeEmitter.event,
		onDidClose: closeEmitter.event,
		open: () => {
			socket = new WebSocket(qjsShellUrl());
			socket.binaryType = "arraybuffer";
			socket.onopen = () => {
				opened = true;
				if (pendingResize) {
					const resize = pendingResize;
					pendingResize = undefined;
					sendResize(resize);
				}
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
						} else if (message.type === "exit") {
							finish(parseTerminalExitCode(message.code));
						}
					} catch {
						// Ignore lifecycle text frames that are not terminal output.
					}
					return;
				}
				const bytes = event.data instanceof Blob
					? await event.data.arrayBuffer()
					: event.data;
				const output = dec.decode(bytes);
				const shellTaskMatch = output.match(/shell task:\s*(\d+)/);
				if (shellTaskMatch && !shellTaskId) {
					shellTaskId = shellTaskMatch[1];
					systemView?.taskStarted(shellTaskId, "shell", "shell");
				}
				writeEmitter.fire(output);
			};
			socket.onerror = () => {
				writeEmitter.fire("\r\nterminal websocket failed\r\n");
			};
			socket.onclose = () => {
				finish();
			};
		},
		close: () => {
			if (closed) {
				return;
			}
			finish();
			socket?.close();
		},
		handleInput: (data: string) => {
			sendInput(enc.encode(data));
			if (data.includes('\r') || data.includes('\n')) {
				notifyFilesystemActivity(onFilesystemActivity);
			}
		},
		setDimensions: (dimensions: vscode.TerminalDimensions) => {
			sendResize(dimensions);
		}
	};
}

type TerminalOptions = {
	keepOpenOnExit?: boolean;
	taskLabel?: string;
	terminalLabel?: string;
	onTaskStarted?: (event: { id: string; kind: string; label: string }) => void;
	onTaskExited?: (event: { id: string; code?: number }) => void;
	onTaskClosed?: (event: { id: string }) => void;
	onTaskOutput?: (event: { id: string; chunk: string }) => void;
	onTerminalOpened?: (event: { id: string; label: string }) => void;
	onTerminalClosed?: (event: { id: string }) => void;
}

async function createTerminal(fsys: any, config: Config, onFilesystemActivity?: () => void, options: TerminalOptions = {}) {
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
	const taskKind = config.shell?.type || "auto";
	const taskLabel = options.taskLabel || config.shell?.cmd || taskKind;
	const terminalLabel = options.terminalLabel || taskLabel;
	options.onTerminalOpened?.({ id: termID, label: terminalLabel });
	options.onTaskStarted?.({ id: taskID, kind: taskKind, label: taskLabel });

	const writeEmitter = new vscode.EventEmitter<string>();
	const closeEmitter = new vscode.EventEmitter<number | void>();
	const dec = new TextDecoder();
	const enc = new TextEncoder();
	const readable = await fsys.openReadable(`${termPath}/data`);
	const reader = readable.getReader();
	const writable = (await fsys.openWritable(`${termPath}/data`)).getWriter();
	let pendingResize: Promise<void> = Promise.resolve();
	let closed = false;
	let taskExitObserved = false;
	let terminalClosed = false;
	let buffer = '';
	let lastOutputAt = Date.now();
	const markTerminalClosed = () => {
		if (terminalClosed) {
			return;
		}
		terminalClosed = true;
		options.onTerminalClosed?.({ id: termID });
	};
	const markTaskClosed = () => {
		if (taskExitObserved) {
			return;
		}
		options.onTaskClosed?.({ id: taskID });
	};
	const emitOutput = (chunk: string) => {
		writeEmitter.fire(chunk);
		options.onTaskOutput?.({ id: taskID, chunk });
	};
	const closeTerminalResource = () => {
		(async () => {
			try {
				const ctl = (await fsys.openWritable(`${termPath}/ctl`)).getWriter();
				try {
					await ctl.write(enc.encode("close"));
				} finally {
					await ctl.close();
				}
			} catch (error) {
				console.warn("Wanix terminal close failed", error);
			}
		})();
	};
	const closeWriter = () => {
		writable.close().catch((error: unknown) => {
			console.warn("Wanix terminal input close failed", error);
		});
	};
	const cancelReader = () => {
		reader.cancel().catch((error: unknown) => {
			console.warn("Wanix terminal output close failed", error);
		});
	};
	const finish = (code?: number) => {
		if (closed) {
			return;
		}
		closed = true;
		markTaskClosed();
		markTerminalClosed();
		closeTerminalResource();
		closeWriter();
		cancelReader();
		closeEmitter.fire(code);
	};
	const finishAndKeepOpen = (code?: number) => {
		if (closed) {
			return;
		}
		closed = true;
		const suffix = typeof code === "number" ? ` ${code}` : "";
		emitOutput(`\r\n[wanix task exited${suffix}]\r\n`);
		markTerminalClosed();
		closeTerminalResource();
		closeWriter();
		cancelReader();
	};
	const watchTaskExit = async () => {
		while (!closed) {
			let exit = "";
			try {
				exit = (await fsys.readText(`${taskPath}/exit`)).trim();
			} catch (error) {
				console.warn("Wanix terminal exit watch failed", error);
			}
			if (exit.length > 0) {
				while (!closed && Date.now() - lastOutputAt < DIRECT_TASK_EXIT_DRAIN_MS) {
					await delay(25);
				}
				notifyFilesystemActivity(onFilesystemActivity);
				const code = parseTerminalExitCode(exit);
				taskExitObserved = true;
				options.onTaskExited?.({ id: taskID, code });
				if (options.keepOpenOnExit) {
					finishAndKeepOpen(code);
				} else {
					finish(code);
				}
				return;
			}
			await delay(DIRECT_TASK_EXIT_POLL_MS);
		}
	};
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
		onDidClose: closeEmitter.event,
		open: () => {
			(async () => {
				try {
					while (!closed) {
						const { done, value } = await reader.read();
						if (done) {
							return;
						}
						lastOutputAt = Date.now();
						emitOutput(dec.decode(value));
					}
				} catch (error) {
					if (!closed) {
						console.warn("Wanix terminal output failed", error);
					}
				} finally {
					reader.releaseLock();
				}
			})();
			watchTaskExit();
		},
		close: () => {
			if (closed) {
				return;
			}
			closed = true;
			markTaskClosed();
			markTerminalClosed();
			closeTerminalResource();
			closeWriter();
			cancelReader();
		},
		handleInput: async (data: string) => {
			if (closed) {
				return;
			}
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
				notifyFilesystemActivity(onFilesystemActivity);
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

function notifyFilesystemActivity(callback?: () => void): void {
	if (!callback) {
		return;
	}
	delay(250).then(callback).catch((error) => {
		console.warn("Wanix filesystem refresh failed", error);
	});
	delay(1000).then(callback).catch((error) => {
		console.warn("Wanix filesystem refresh failed", error);
	});
}

function parseTerminalExitCode(value: unknown): number | undefined {
	if (typeof value === "number" && Number.isInteger(value)) {
		return value;
	}
	if (typeof value !== "string") {
		return undefined;
	}
	const text = value.trim();
	if (text.length === 0) {
		return undefined;
	}
	const code = Number.parseInt(text, 10);
	return Number.isInteger(code) ? code : undefined;
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

function safeOutputFileName(name: string): string {
	const safe = name.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/^_+|_+$/g, "");
	return (safe || "task").slice(0, 80);
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
