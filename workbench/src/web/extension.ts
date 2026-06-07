
import * as vscode from 'vscode';
import { AGENT_BROKEN_PATH, installAgentRepairDemo, repairQjsProgram } from './agent-repair-demo.js';
import { WanixBridge, type WanixBridgeMutation } from './bridge.js';
import { DUET_DEMO_STEPS, DUET_OUTPUT_PATH, installDuetDemo, resetDuetDemo } from './duet-demo.js';
import { copyHttpAppUrl, installHttpAppDemo, openHttpAppDemo, openHttpAppHandler, openHttpCounterDemo, openHttpWasmDemo, type HttpAppRouteConfig } from './http-app-demo.js';
import { createQjsStarter } from './qjs-starter.js';
import { WANIX_INSPECT_SCHEME, WanixServiceInspector } from './service-inspector.js';
import { WanixSystemView, type WanixServiceTask, type WanixServiceTerminal } from './system-view.js';
import { openDirectV86, openV86SharedDemo, V86_SHARED_DIR, V86_SHARED_LINUX_PATH, type V86SharedConfig } from './v86-shared-demo.js';
import { ensureWasmStarter, installWasmStarter, WASM_STARTER_OUTPUT_PATH, WASM_STARTER_PATH } from './wasm-starter.js';
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
	v86?: V86SharedConfig;
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

type TaskRunStart = {
	taskId: string;
	outputPath?: string;
	metadataPath?: string;
};

type TaskArtifacts = {
	outputPath: string;
	metadataPath: string;
};

type AgentTraceStep = {
	label: string;
	description?: string;
	path?: string;
	beforePath?: string;
	afterPath?: string;
	taskId?: string;
	outputPath?: string;
	metadataPath?: string;
	exitCode?: number;
};

type FilesystemActivity = {
	label?: string;
	openPath?: string;
	paths?: string[];
};

const TASK_RUNNERS: Record<TaskRunKind, { extension: string; label: string }> = {
	qjs: { extension: ".js", label: "JavaScript" },
	wasm: { extension: ".wasm", label: "WASM" },
};
const TASK_OUTPUT_DIR = ".wanix/tasks";
const TASK_OUTPUT_MAX_CHARS = 512 * 1024;
const COCKPIT_TOUR_REPORT_PATH = ".wanix/cockpit-tour.md";
const SYSTEM_JOURNAL_PATH = ".wanix/system-journal.md";
const SYSTEM_STATE_PATH = ".wanix/system-state.json";
const SERVICE_STATE_POLL_MS = 1000;
const SHARED_DIRECTORY_POLL_MS = 1500;

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
	const refreshFiles = async (activity?: FilesystemActivity) => {
		if (activity?.paths?.length) {
			await refreshWanixPaths(bridge, activity.paths);
			systemView.filesystemActivity(activity.label || "filesystem refreshed", {
				path: activity.openPath,
				paths: activity.paths,
			});
			return;
		}
		await refreshWorkbenchFiles(bridge);
		systemView.filesystemActivity(activity?.label || "filesystem refreshed");
	};
	const activeTaskTerminals = new Map<TaskRunKind, vscode.Terminal>();
	const taskTerminals = new Map<string, vscode.Terminal>();
	let sharedWatcher: WanixSharedDirectoryWatcher | undefined;
	context.subscriptions.push(bridge);
	context.subscriptions.push(bridge.onDidWanixMutation((mutation) => {
		const activity = bridgeMutationActivity(mutation);
		systemView.filesystemActivity(activity.label, {
			path: activity.openPath,
			paths: activity.paths,
		});
	}));
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
		const serviceInspector = new WanixServiceInspector(fsys, bridge);
		context.subscriptions.push(
			serviceInspector,
			vscode.workspace.registerTextDocumentContentProvider(WANIX_INSPECT_SCHEME, serviceInspector),
			vscode.languages.registerDocumentLinkProvider({ scheme: WANIX_INSPECT_SCHEME }, serviceInspector),
		);
		systemView.configure(config);
		context.subscriptions.push(startWanixServiceStatePolling(fsys, config, systemView));
		sharedWatcher = new WanixSharedDirectoryWatcher(fsys, bridge, systemView);
		context.subscriptions.push(sharedWatcher);
		if (config.v86?.launchUrl) {
			sharedWatcher.start();
		}
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
		context.subscriptions.push(vscode.commands.registerCommand('workbench.resetDuetDemo', async () => {
			try {
				await resetDuetDemo(context, fsys, bridge, systemView);
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
		context.subscriptions.push(vscode.commands.registerCommand('workbench.runCockpitTour', async () => {
			try {
				await runCockpitTour(fsys, bridge, config, systemView, activeTaskTerminals, taskTerminals, context, sharedWatcher);
				revealWanixSystemView();
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
		context.subscriptions.push(vscode.commands.registerCommand('workbench.installWasmStarter', async () => {
			try {
				await installWasmStarter(context, fsys, bridge, systemView);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.runWasmStarter', async () => {
			try {
				await runWasmStarter(fsys, bridge, config, systemView, activeTaskTerminals, taskTerminals, context);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openV86SharedDemo', async () => {
			try {
				await openV86SharedDemo(fsys, bridge, config, systemView);
				await sharedWatcher?.resetBaseline();
				sharedWatcher?.start();
				systemView.filesystemActivity("v86 shared watch armed", {
					path: V86_SHARED_LINUX_PATH,
					paths: [V86_SHARED_DIR, V86_SHARED_LINUX_PATH],
				});
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openDirectV86', async () => {
			try {
				await openDirectV86(config, systemView);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.installAgentRepairDemo', async () => {
			try {
				await installAgentRepairDemo(fsys, bridge, systemView);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.runAgentRepairDemo', async () => {
			try {
				await runAgentRepairDemo(fsys, bridge, config, systemView, activeTaskTerminals, taskTerminals, context);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.fixCurrentWanixProgram', async () => {
			try {
				await fixCurrentWanixProgram(fsys, bridge, config, systemView, activeTaskTerminals, taskTerminals, context);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.clearFinishedSystemRows', () => {
			systemView.clearFinishedRows();
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openSystemJournal', async () => {
			try {
				await openSystemJournal(fsys, bridge, systemView);
				revealWanixSystemView();
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
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openWanixPath', async (target?: string | { path?: string }) => {
			try {
				const path = wanixPathTarget(target);
				if (!path) {
					throw new Error("No Wanix path available to open");
				}
				await openWanixPathOrReveal(fsys, bridge, serviceInspector, systemView, path);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openWanixDiff', async (target?: { beforePath?: string; afterPath?: string }) => {
			try {
				const diff = wanixDiffTarget(target);
				if (!diff) {
					throw new Error("No Wanix diff paths available to open");
				}
				await openWanixDiff(diff.beforePath, diff.afterPath);
				systemView.filesystemActivity(`opened diff ${baseName(diff.afterPath)}`);
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
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openWanixTaskMetadata', async (metadata?: string | { metadataPath?: string }) => {
			try {
				const metadataPath = taskMetadataPath(metadata);
				if (!metadataPath) {
					throw new Error("Task metadata is not available");
				}
				await openWanixPath(metadataPath);
				systemView.filesystemActivity(`opened task metadata ${baseName(metadataPath)}`);
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
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openHttpCounterDemo', async () => {
			try {
				await openHttpCounterDemo(fsys, bridge, config, systemView);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openHttpWasmDemo', async () => {
			try {
				await openHttpWasmDemo(fsys, bridge, config, systemView);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openHttpAppPreview', async (target?: string | { path?: string }) => {
			try {
				const path = wanixPathTarget(target);
				if (!path) {
					throw new Error("No HTTP app preview has been saved yet");
				}
				await openWanixPath(path);
				systemView.filesystemActivity(`opened http app preview ${baseName(path)}`);
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
	await openWanixUri(wanixUri(path));
}

async function openWanixDiff(beforePath: string, afterPath: string): Promise<void> {
	await vscode.commands.executeCommand(
		"vscode.diff",
		wanixUri(beforePath),
		wanixUri(afterPath),
		`Agent repair: ${baseName(afterPath)}`,
		{ preview: false },
	);
	rememberWanixEditor();
}

async function openWanixPathOrReveal(
	fsys: any,
	bridge: WanixBridge,
	serviceInspector: WanixServiceInspector,
	systemView: WanixSystemView,
	path: string,
): Promise<void> {
	const uri = wanixUri(path);
	const fsPath = bridge.normalizePath(uri.path);
	const stat = await fsys.stat(fsPath);
	if (stat?.IsDir) {
		await serviceInspector.open(path);
		systemView.filesystemActivity(`inspected ${fsPath}`);
		return;
	}
	await openWanixUri(uri);
	systemView.filesystemActivity(`opened ${fsPath}`);
}

async function openSystemJournal(fsys: any, bridge: WanixBridge, systemView: WanixSystemView): Promise<void> {
	await fsys.makeDirAll(".wanix");
	const generatedAt = new Date();
	systemView.filesystemActivity("system state written", { path: SYSTEM_STATE_PATH });
	systemView.filesystemActivity("system journal written", { path: SYSTEM_JOURNAL_PATH });
	await fsys.writeFile(SYSTEM_JOURNAL_PATH, systemView.systemJournalMarkdown({
		generatedAt,
		path: SYSTEM_JOURNAL_PATH,
		statePath: SYSTEM_STATE_PATH,
	}));
	await fsys.writeFile(SYSTEM_STATE_PATH, systemView.systemStateJson({
		generatedAt,
		journalPath: SYSTEM_JOURNAL_PATH,
		statePath: SYSTEM_STATE_PATH,
	}));
	await refreshWanixPaths(bridge, [SYSTEM_JOURNAL_PATH, SYSTEM_STATE_PATH]);
	await openWanixPath(SYSTEM_JOURNAL_PATH);
	vscode.window.showInformationMessage("Opened Wanix system journal");
}

function wanixUri(path: string): vscode.Uri {
	return vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path: absoluteWanixPath(path),
	});
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

function taskMetadataPath(metadata: string | { metadataPath?: string } | undefined): string | undefined {
	if (typeof metadata === "string") {
		return metadata;
	}
	return metadata?.metadataPath;
}

function wanixPathTarget(target: string | { path?: string } | undefined): string | undefined {
	if (typeof target === "string") {
		return target;
	}
	return target?.path;
}

function wanixDiffTarget(target: { beforePath?: string; afterPath?: string } | undefined): { beforePath: string; afterPath: string } | undefined {
	if (!target?.beforePath || !target.afterPath) {
		return undefined;
	}
	return {
		beforePath: target.beforePath,
		afterPath: target.afterPath,
	};
}

function bridgeMutationActivity(mutation: WanixBridgeMutation): Required<FilesystemActivity> {
	const paths = mutation.paths;
	const openPath = mutation.openPath || paths[paths.length - 1] || "/";
	const primary = paths[0] || openPath;
	const target = paths[paths.length - 1] || openPath;
	switch (mutation.kind) {
		case "copy":
			return { label: `copied ${baseName(primary)} -> ${baseName(target)}`, openPath, paths };
		case "rename":
			return { label: `renamed ${baseName(primary)} -> ${baseName(target)}`, openPath, paths };
		case "delete":
			return { label: `deleted ${baseName(primary)}`, openPath, paths };
		case "mkdir":
			return { label: `created directory ${baseName(target)}`, openPath, paths };
		case "write":
			return { label: `saved ${baseName(target)}`, openPath, paths };
	}
}

function startWanixServiceStatePolling(fsys: any, config: Config, systemView: WanixSystemView): vscode.Disposable {
	let disposed = false;
	let inFlight = false;
	let warned = false;
	const poll = async () => {
		if (disposed || inFlight || !config.ns?.task || !config.ns?.term) {
			return;
		}
		inFlight = true;
		try {
			systemView.observeServiceState(await readWanixServiceState(fsys, config));
			warned = false;
		} catch (error) {
			if (!warned) {
				console.warn("Wanix service state poll failed", error);
				warned = true;
			}
		} finally {
			inFlight = false;
		}
	};
	const interval = setInterval(() => {
		void poll();
	}, SERVICE_STATE_POLL_MS);
	void poll();
	return new vscode.Disposable(() => {
		disposed = true;
		clearInterval(interval);
	});
}

async function readWanixServiceState(fsys: any, config: Config): Promise<{ tasks: WanixServiceTask[]; terminals: WanixServiceTerminal[] }> {
	const [tasks, terminals] = await Promise.all([
		readWanixServiceTasks(fsys, config.ns?.task),
		readWanixServiceTerminals(fsys, config.ns?.term),
	]);
	return { tasks, terminals };
}

async function readWanixServiceTasks(fsys: any, taskRoot: string | undefined): Promise<WanixServiceTask[]> {
	if (!taskRoot) {
		return [];
	}
	let names: string[];
	try {
		names = serviceEntryNames(await fsys.readDir(taskRoot));
	} catch {
		return [];
	}
	const ids = names.filter((name) => /^\d+$/.test(name));
	const tasks = await Promise.all(ids.map((id) => readWanixServiceTask(fsys, taskRoot, id)));
	return tasks.filter((task): task is WanixServiceTask => Boolean(task));
}

async function readWanixServiceTask(fsys: any, taskRoot: string, id: string): Promise<WanixServiceTask | undefined> {
	const taskPath = `${taskRoot}/${id}`;
	try {
		const [kindText, cmdText, exitText] = await Promise.all([
			readWanixServiceText(fsys, `${taskPath}/kind`),
			readWanixServiceText(fsys, `${taskPath}/cmd`),
			readWanixServiceText(fsys, `${taskPath}/exit`),
		]);
		const kind = kindText.trim() || "task";
		const cmd = cmdText.trim();
		const exit = exitText.trim();
		return {
			id,
			kind,
			label: serviceTaskLabel(kind, cmd),
			exit,
			exitCode: parseTerminalExitCode(exit),
		};
	} catch {
		return undefined;
	}
}

async function readWanixServiceTerminals(fsys: any, termRoot: string | undefined): Promise<WanixServiceTerminal[]> {
	if (!termRoot) {
		return [];
	}
	let names: string[];
	try {
		names = serviceEntryNames(await fsys.readDir(termRoot));
	} catch {
		return [];
	}
	return names
		.filter((name) => /^\d+$/.test(name))
		.map((id) => ({ id, label: `term ${id}` }));
}

async function readWanixServiceText(fsys: any, path: string): Promise<string> {
	const value = await fsys.readText(path);
	return typeof value === "string" ? value : String(value);
}

function serviceEntryNames(entries: unknown): string[] {
	if (!Array.isArray(entries)) {
		return [];
	}
	return entries
		.map((entry) => {
			if (typeof entry === "string") {
				return entry.replace(/\/$/, "");
			}
			if (entry && typeof entry === "object") {
				const candidate = entry as { Name?: string; name?: string };
				return (candidate.Name || candidate.name || "").replace(/\/$/, "");
			}
			return "";
		})
		.filter((name) => name.length > 0)
		.sort((left, right) => left.localeCompare(right, undefined, { numeric: true }));
}

function serviceTaskLabel(kind: string, cmd: string): string {
	const words = shellWords(cmd);
	if (words[0]) {
		return baseName(words[0]);
	}
	return kind;
}

type SharedDirectoryEntry = {
	path: string;
	isDir: boolean;
	size: number;
	modTime: number;
};

class WanixSharedDirectoryWatcher implements vscode.Disposable {
	private baseline = new Map<string, string>();
	private baselineInitialized = false;
	private interval: ReturnType<typeof setInterval> | undefined;
	private started = false;
	private inFlight = false;
	private warned = false;

	constructor(
		private readonly fsys: any,
		private readonly bridge: WanixBridge,
		private readonly systemView: WanixSystemView,
	) {}

	start(): void {
		if (this.started) {
			return;
		}
		this.started = true;
		this.interval = setInterval(() => {
			void this.poll();
		}, SHARED_DIRECTORY_POLL_MS);
		void this.poll();
	}

	async resetBaseline(): Promise<void> {
		this.baseline = await this.snapshot();
		this.baselineInitialized = true;
	}

	dispose(): void {
		this.started = false;
		if (this.interval) {
			clearInterval(this.interval);
			this.interval = undefined;
		}
	}

	private async poll(): Promise<void> {
		if (!this.started || this.inFlight) {
			return;
		}
		this.inFlight = true;
		try {
			const next = await this.snapshot();
			if (!this.baselineInitialized) {
				this.baseline = next;
				this.baselineInitialized = true;
				this.warned = false;
				return;
			}
			const changes = sharedDirectoryChanges(this.baseline, next);
			this.baseline = next;
			if (changes.length > 0) {
				await refreshWanixPaths(this.bridge, changes);
				for (const path of changes) {
					this.systemView.filesystemActivity(sharedDirectoryActivityLabel(path), {
						path,
						paths: [path],
					});
				}
				revealWanixSystemView();
			}
			this.warned = false;
		} catch (error) {
			if (!this.warned) {
				console.warn("Wanix shared directory watch failed", error);
				this.warned = true;
			}
		} finally {
			this.inFlight = false;
		}
	}

	private async snapshot(): Promise<Map<string, string>> {
		const entries = await readSharedDirectoryEntries(this.fsys);
		return new Map(entries.map((entry) => [entry.path, sharedEntrySignature(entry)]));
	}
}

async function readSharedDirectoryEntries(fsys: any): Promise<SharedDirectoryEntry[]> {
	let entries: unknown;
	try {
		entries = typeof fsys.readDirEntries === "function"
			? await fsys.readDirEntries(V86_SHARED_DIR)
			: await fsys.readDir(V86_SHARED_DIR);
	} catch {
		return [];
	}
	const names = serviceEntryNames(entries);
	const results: SharedDirectoryEntry[] = [];
	for (const name of names) {
		const path = `${V86_SHARED_DIR}/${name}`;
		try {
			const stat = await fsys.stat(path);
			results.push({
				path,
				isDir: Boolean(stat?.IsDir),
				size: Number(stat?.Size || 0),
				modTime: Number(stat?.ModTime || 0),
			});
		} catch {
			// The file can disappear between readdir and stat.
		}
	}
	return results;
}

function sharedDirectoryChanges(previous: Map<string, string>, next: Map<string, string>): string[] {
	const changes: string[] = [];
	for (const [path, signature] of next) {
		if (previous.get(path) !== signature) {
			changes.push(path);
		}
	}
	for (const path of previous.keys()) {
		if (!next.has(path)) {
			changes.push(path);
		}
	}
	return changes.sort((left, right) => left.localeCompare(right));
}

function sharedEntrySignature(entry: SharedDirectoryEntry): string {
	return `${entry.isDir ? "dir" : "file"}:${entry.size}:${entry.modTime}`;
}

function sharedDirectoryActivityLabel(path: string): string {
	if (path === V86_SHARED_LINUX_PATH) {
		return "v86 shared file changed from-linux.txt";
	}
	return `v86 shared file changed ${baseName(path)}`;
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

async function runWasmStarter(
	fsys: any,
	bridge: WanixBridge,
	config: Config,
	systemView: WanixSystemView,
	activeTaskTerminals: Map<TaskRunKind, vscode.Terminal>,
	taskTerminals: Map<string, vscode.Terminal>,
	context: vscode.ExtensionContext,
): Promise<void> {
	await ensureWasmStarter(context, fsys, bridge);
	systemView.filesystemActivity("wasm starter run started");
	const code = await runWanixTaskTarget(
		fsys,
		bridge,
		config,
		systemView,
		activeTaskTerminals,
		taskTerminals,
		"wasm",
		taskRunTargetFromPath(bridge, "wasm", WASM_STARTER_PATH),
		context,
		{ waitForExit: true },
	);
	if (code !== 0) {
		throw new Error(`WASM starter exited ${formatTaskExitCode(code)}`);
	}
	systemView.filesystemActivity("wasm starter output opened");
	await openWanixPath(WASM_STARTER_OUTPUT_PATH);
	vscode.window.showInformationMessage("Wanix WASM starter completed");
}

type CockpitTourStep = {
	label: string;
	status: "ok" | "failed";
	description: string;
	artifacts: string[];
	error?: string;
};

async function runCockpitTour(
	fsys: any,
	bridge: WanixBridge,
	config: Config,
	systemView: WanixSystemView,
	activeTaskTerminals: Map<TaskRunKind, vscode.Terminal>,
	taskTerminals: Map<string, vscode.Terminal>,
	context: vscode.ExtensionContext,
	sharedWatcher?: WanixSharedDirectoryWatcher,
): Promise<void> {
	const startedAt = new Date();
	const steps: CockpitTourStep[] = [];
	const runStep = async (
		label: string,
		description: string,
		artifacts: string[],
		action: () => Promise<void>,
	): Promise<void> => {
		systemView.tourStepStarted(label, { description, artifacts });
		systemView.filesystemActivity(`cockpit tour ${label}`);
		try {
			await action();
			steps.push({ label, description, artifacts, status: "ok" });
			systemView.tourStepCompleted(label);
		} catch (error) {
			steps.push({
				label,
				description,
				artifacts,
				status: "failed",
				error: error instanceof Error ? error.message : String(error),
			});
			systemView.tourStepFailed(label, error);
			throw error;
		}
	};

	systemView.tourStarted("OS cockpit tour");
	systemView.filesystemActivity("cockpit tour started");
	try {
		await runStep(
			"seed v86 shared files",
			"Create the Linux/v86 shared-file workspace and arm the browser-side shared directory watcher.",
			["/shared/README.md", "/shared/message.txt", V86_SHARED_LINUX_PATH],
			async () => {
				await openV86SharedDemo(fsys, bridge, config, systemView, { openReadme: false, notify: false });
				await sharedWatcher?.resetBaseline();
				sharedWatcher?.start();
				systemView.filesystemActivity("v86 shared watch armed", {
					path: V86_SHARED_LINUX_PATH,
					paths: [V86_SHARED_DIR, V86_SHARED_LINUX_PATH],
				});
			},
		);
		await runStep(
			"run JS and WASM duet",
			"Run qjs producer, compiled WASM transform, and qjs verifier through one Wanix namespace.",
			["/duet/producer.js", "/duet/transform.wasm", "/duet/verify.js", DUET_OUTPUT_PATH],
			() => runDuetDemo(fsys, bridge, config, systemView, activeTaskTerminals, taskTerminals, context),
		);
		await runStep(
			"preview stateful HTTP route",
			"Run a qjs-backed Wanix HTTP route and preserve its response plus state file.",
			["/apps/counter.js", "/apps/counter.response.txt", "/apps/counter.count.txt"],
			() => openHttpCounterDemo(fsys, bridge, config, systemView),
		);
		await runStep(
			"preview WASM HTTP route",
			"Run a compiled WASM handler through the same Wanix HTTP route contract.",
			["/apps/wasm.wasm", "/apps/wasm.response.txt"],
			() => openHttpWasmDemo(fsys, bridge, config, systemView),
		);
		await runStep(
			"run agent repair",
			"Install, fail, repair, rerun, and report the broken qjs program using Wanix-visible operations.",
			["/agent/broken.js", "/agent/out/broken.repair-report.md", "/agent/out/result.txt"],
			() => runAgentRepairDemo(fsys, bridge, config, systemView, activeTaskTerminals, taskTerminals, context),
		);
		const reportPath = await writeCockpitTourReport(fsys, bridge, startedAt, new Date(), "complete", steps);
		systemView.tourStepCompleted("OS cockpit tour");
		systemView.tourReport(reportPath, "complete");
		systemView.filesystemActivity("cockpit tour report written", { path: reportPath });
		await refreshWanixPaths(bridge, [reportPath]);
		await openWanixPath(reportPath);
		vscode.window.showInformationMessage("Wanix OS cockpit tour completed");
	} catch (error) {
		const reportPath = await writeCockpitTourReport(fsys, bridge, startedAt, new Date(), "failed", steps, error);
		systemView.tourStepFailed("OS cockpit tour", error);
		systemView.tourReport(reportPath, "failed");
		systemView.filesystemActivity("cockpit tour failure report written", { path: reportPath });
		await refreshWanixPaths(bridge, [reportPath]);
		await openWanixPath(reportPath);
		throw error;
	}
}

async function fixCurrentWanixProgram(
	fsys: any,
	bridge: WanixBridge,
	config: Config,
	systemView: WanixSystemView,
	activeTaskTerminals: Map<TaskRunKind, vscode.Terminal>,
	taskTerminals: Map<string, vscode.Terminal>,
	context: vscode.ExtensionContext,
): Promise<void> {
	const target = await taskRunTarget("qjs", bridge);
	await repairWanixProgramTarget(fsys, bridge, config, systemView, activeTaskTerminals, taskTerminals, target, context);
}

async function runAgentRepairDemo(
	fsys: any,
	bridge: WanixBridge,
	config: Config,
	systemView: WanixSystemView,
	activeTaskTerminals: Map<TaskRunKind, vscode.Terminal>,
	taskTerminals: Map<string, vscode.Terminal>,
	context: vscode.ExtensionContext,
): Promise<void> {
	await installAgentRepairDemo(fsys, bridge, systemView);
	await repairWanixProgramTarget(
		fsys,
		bridge,
		config,
		systemView,
		activeTaskTerminals,
		taskTerminals,
		taskRunTargetFromPath(bridge, "qjs", AGENT_BROKEN_PATH),
		context,
	);
}

async function repairWanixProgramTarget(
	fsys: any,
	bridge: WanixBridge,
	config: Config,
	systemView: WanixSystemView,
	activeTaskTerminals: Map<TaskRunKind, vscode.Terminal>,
	taskTerminals: Map<string, vscode.Terminal>,
	target: TaskRunTarget,
	context: vscode.ExtensionContext,
): Promise<void> {
	const trace: AgentTraceStep[] = [];
	const startedAt = new Date();
	let reportPath: string | undefined;
	const agentStep = (label: string, options: Partial<AgentTraceStep> & { icon?: string } = {}): void => {
		trace.push({
			label,
			description: options.description,
			path: options.path,
			beforePath: options.beforePath,
			afterPath: options.afterPath,
			taskId: options.taskId,
			outputPath: options.outputPath,
			metadataPath: options.metadataPath,
			exitCode: options.exitCode,
		});
		systemView.agentStep(label, {
			description: options.description,
			icon: options.icon,
			path: options.path,
			beforePath: options.beforePath,
			afterPath: options.afterPath,
		});
	};
	const writeReport = async (status: string, error?: unknown, resultPath?: string): Promise<string> => {
		const path = reportPath || agentReportPath(target);
		reportPath = path;
		const reportDir = parentPath(path);
		if (reportDir) {
			await fsys.makeDirAll(reportDir);
		}
		const report = agentRepairReportMarkdown({
			target,
			status,
			startedAt,
			completedAt: new Date(),
			steps: trace,
			reportPath: path,
			resultPath,
			error: error instanceof Error ? error.message : error ? String(error) : undefined,
		});
		await fsys.writeFile(path, report);
		bridge.refresh(path);
		return path;
	};
	systemView.agentStarted(`repair ${target.name}`);
	try {
		agentStep(`read ${target.name}`, { icon: "book", path: target.path });
		const source = await fsys.readText(target.path);
		let firstRun: TaskRunStart | undefined;
		agentStep(`run qjs ${target.name}`, { icon: "play" });
		const firstCode = await runWanixTaskTarget(
			fsys,
			bridge,
			config,
			systemView,
			activeTaskTerminals,
			taskTerminals,
			"qjs",
			target,
			context,
			{
				waitForExit: true,
				onTaskStarted: (event) => {
					firstRun = event;
					if (event.outputPath) {
						agentStep("capture first transcript", {
							icon: "output",
							path: event.outputPath,
							taskId: event.taskId,
							outputPath: event.outputPath,
							metadataPath: event.metadataPath,
						});
					}
				},
			},
		);
		const firstTranscript = firstRun?.outputPath
			? await waitForTaskTranscript(fsys, firstRun.outputPath)
			: "";
		const observation = agentObservation(firstCode, firstTranscript);
		agentStep(`observe ${observation}`, { icon: firstCode === 0 ? "pass" : "warning", exitCode: firstCode });
		const repaired = repairQjsProgram(source);
		if (repaired === source && firstCode === 0) {
			agentStep("already repaired", { icon: "pass", path: target.path });
			const path = await writeReport("already repaired");
			agentStep("write repair report", { icon: "notebook", path });
			await openWanixPath(path);
			vscode.window.showInformationMessage(`${target.name} already looks repaired`);
			return;
		}
		const diff = agentDiffPaths(target);
		await fsys.makeDirAll(diff.dir);
		await fsys.writeFile(diff.beforePath, source);
		agentStep("snapshot original", { icon: "go-to-file", path: diff.beforePath });
		await fsys.writeFile(target.path, repaired);
		await fsys.writeFile(diff.afterPath, repaired);
		bridge.refresh(target.path);
		bridge.refresh(diff.beforePath);
		bridge.refresh(diff.afterPath);
		await refreshWorkbenchFiles(bridge);
		agentStep(`edit ${target.name}`, { icon: "edit", path: target.path });
		await openWanixPath(target.path);
		agentStep(`diff ${target.name}`, {
			description: `${baseName(diff.beforePath)} -> ${baseName(diff.afterPath)}`,
			icon: "diff",
			beforePath: diff.beforePath,
			afterPath: diff.afterPath,
		});
		await openWanixDiff(diff.beforePath, diff.afterPath);
		let secondRun: TaskRunStart | undefined;
		agentStep(`rerun qjs ${target.name}`, { icon: "run" });
		const secondCode = await runWanixTaskTarget(
			fsys,
			bridge,
			config,
			systemView,
			activeTaskTerminals,
			taskTerminals,
			"qjs",
			target,
			context,
			{
				waitForExit: true,
				onTaskStarted: (event) => {
					secondRun = event;
					if (event.outputPath) {
						agentStep("capture rerun transcript", {
							icon: "output",
							path: event.outputPath,
							taskId: event.taskId,
							outputPath: event.outputPath,
							metadataPath: event.metadataPath,
						});
					}
				},
			},
		);
		if (secondRun?.outputPath) {
			await waitForTaskTranscript(fsys, secondRun.outputPath);
		}
		if (secondCode !== 0) {
			agentStep(`rerun failed ${formatTaskExitCode(secondCode)}`, { icon: "error", exitCode: secondCode });
			throw new Error(`Agent repair rerun exited ${formatTaskExitCode(secondCode)}`);
		}
		const resultPath = agentResultPath(target);
		const result = await waitForTextFile(fsys, resultPath);
		agentStep(`verify ${baseName(resultPath)}`, { icon: "pass", path: resultPath });
		await openWanixPath(resultPath);
		const path = await writeReport("repaired", undefined, resultPath);
		agentStep("write repair report", { icon: "notebook", path });
		await refreshWanixPaths(bridge, [resultPath, path]);
		await openWanixPath(path);
		vscode.window.showInformationMessage(`Wanix agent repair wrote ${resultPath}: ${result.trim()}`);
	} catch (error) {
		agentStep("repair failed", { icon: "error", description: error instanceof Error ? error.message : String(error) });
		try {
			const path = await writeReport("failed", error);
			agentStep("write failure report", { icon: "notebook", path });
			await openWanixPath(path);
		} catch (reportError) {
			console.warn("Wanix agent repair report failed", reportError);
		}
		throw error;
	}
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
	options: { waitForExit?: boolean; onTaskStarted?: (event: TaskRunStart) => void } = {},
): Promise<number | undefined> {
	activeTaskTerminals.get(kind)?.dispose();
	activeTaskTerminals.delete(kind);
	let resolveExit: (code: number | undefined) => void = () => {};
	const exit = new Promise<number | undefined>((resolve) => {
		resolveExit = resolve;
	});
	let startedTaskId: string | undefined;
	const pty = await createActiveTaskTerminal(fsys, bridge, config, systemView, kind, target, {
		onStart: (event) => {
			startedTaskId = event.taskId;
			options.onTaskStarted?.(event);
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
	lifecycle: { onStart?: (event: TaskRunStart) => void; onExit?: (code: number | undefined) => void; onClose?: () => void } = {},
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
			const artifacts = output.start(event.id);
			lifecycle.onStart?.({ taskId: event.id, ...artifacts });
			systemView.taskStarted(event.id, event.kind, event.label, { sourcePath: target.path, ...artifacts });
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
	private taskId: string | undefined;
	private outputPath: string | undefined;
	private metadataPath: string | undefined;

	constructor(
		private readonly fsys: any,
		private readonly bridge: WanixBridge,
		private readonly systemView: WanixSystemView,
		private readonly kind: TaskRunKind,
		private readonly target: TaskRunTarget,
	) {}

	start(taskId: string): TaskArtifacts {
		this.taskId = taskId;
		this.outputPath = `${TASK_OUTPUT_DIR}/${taskId}-${this.kind}-${safeOutputFileName(this.target.name)}.output.txt`;
		this.metadataPath = `${TASK_OUTPUT_DIR}/${taskId}-${this.kind}-${safeOutputFileName(this.target.name)}.metadata.json`;
		return {
			outputPath: this.outputPath,
			metadataPath: this.metadataPath,
		};
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
		if (this.saved || !this.outputPath || !this.metadataPath) {
			return;
		}
		this.saved = true;
		await this.fsys.makeDirAll(TASK_OUTPUT_DIR);
		await this.fsys.writeFile(this.outputPath, this.render(exitCode));
		await this.fsys.writeFile(this.metadataPath, this.renderMetadata(exitCode));
		this.bridge.refresh(`/${TASK_OUTPUT_DIR}`);
		this.bridge.refresh(`/${this.outputPath}`);
		this.bridge.refresh(`/${this.metadataPath}`);
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

	private renderMetadata(exitCode?: number): string {
		const status = typeof exitCode === "number" ? "exited" : "closed";
		return `${JSON.stringify({
			taskId: this.taskId,
			kind: this.kind,
			label: this.target.name,
			argv: [this.target.name],
			cwd: this.target.dir,
			env: {},
			sourcePath: absoluteWanixPath(this.target.path),
			outputPath: this.outputPath ? absoluteWanixPath(this.outputPath) : undefined,
			status,
			exitCode: typeof exitCode === "number" ? exitCode : null,
			transcript: {
				truncated: this.truncated,
				maxChars: TASK_OUTPUT_MAX_CHARS,
				capturedChars: this.totalChars,
			},
			recordedAt: new Date().toISOString(),
		}, null, 2)}\n`;
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

async function waitForTaskTranscript(fsys: any, path: string): Promise<string> {
	return await waitForTextFile(fsys, path, 3000);
}

async function waitForTextFile(fsys: any, path: string, timeoutMs = 3000): Promise<string> {
	const start = Date.now();
	let lastError: unknown;
	while (Date.now() - start <= timeoutMs) {
		try {
			return await fsys.readText(path);
		} catch (error) {
			lastError = error;
			await delay(100);
		}
	}
	throw new Error(`Timed out waiting for ${path}: ${lastError instanceof Error ? lastError.message : String(lastError)}`);
}

function agentObservation(code: number | undefined, transcript: string): string {
	const referenceError = transcript.match(/ReferenceError[^\r\n]*/)?.[0];
	if (referenceError) {
		return referenceError;
	}
	return `exit ${formatTaskExitCode(code)}`;
}

function agentResultPath(target: TaskRunTarget): string {
	const dir = target.dir === "." ? "" : target.dir.replace(/^\/+|\/+$/g, "");
	return dir ? `${dir}/out/result.txt` : "out/result.txt";
}

function agentDiffPaths(target: TaskRunTarget): { dir: string; beforePath: string; afterPath: string } {
	const dir = target.dir === "." ? "" : target.dir.replace(/^\/+|\/+$/g, "");
	const artifactDir = dir ? `${dir}/out` : "out";
	const stem = safeOutputFileName(target.name.replace(/\.js$/i, ""));
	return {
		dir: artifactDir,
		beforePath: `${artifactDir}/${stem}.before.js`,
		afterPath: `${artifactDir}/${stem}.after.js`,
	};
}

function agentReportPath(target: TaskRunTarget): string {
	const dir = target.dir === "." ? "" : target.dir.replace(/^\/+|\/+$/g, "");
	const artifactDir = dir ? `${dir}/out` : "out";
	const stem = safeOutputFileName(target.name.replace(/\.js$/i, ""));
	return `${artifactDir}/${stem}.repair-report.md`;
}

function agentRepairReportMarkdown(report: {
	target: TaskRunTarget;
	status: string;
	startedAt: Date;
	completedAt: Date;
	steps: AgentTraceStep[];
	reportPath: string;
	resultPath?: string;
	error?: string;
}): string {
	const artifacts = uniqueReportPaths([
		report.target.path,
		report.resultPath || "",
		report.reportPath,
		...report.steps.flatMap((step) => [
			step.path || "",
			step.beforePath || "",
			step.afterPath || "",
			step.outputPath || "",
			step.metadataPath || "",
		]),
	]);
	return [
		"# Wanix Agent Repair Report",
		"",
		`Status: ${report.status}`,
		`Target: ${displayWanixReportPath(report.target.path)}`,
		"Backend: deterministic local repair",
		"Contract: read file, run task, observe transcript, write file, rerun task, verify filesystem output",
		`Started: ${report.startedAt.toISOString()}`,
		`Completed: ${report.completedAt.toISOString()}`,
		report.error ? `Error: ${report.error}` : undefined,
		"",
		"## Operations",
		"",
		...report.steps.flatMap((step, index) => agentReportStepLines(step, index + 1)),
		"## Artifacts",
		"",
		...artifacts.map((path) => `- ${displayWanixReportPath(path)}`),
		"",
	].filter((line): line is string => line !== undefined).join("\n");
}

function agentReportStepLines(step: AgentTraceStep, index: number): string[] {
	const details = [
		step.description ? `detail: ${step.description}` : undefined,
		step.taskId ? `task: ${step.taskId}` : undefined,
		typeof step.exitCode === "number" ? `exit: ${step.exitCode}` : undefined,
		step.path ? `path: ${displayWanixReportPath(step.path)}` : undefined,
		step.beforePath ? `before: ${displayWanixReportPath(step.beforePath)}` : undefined,
		step.afterPath ? `after: ${displayWanixReportPath(step.afterPath)}` : undefined,
		step.outputPath ? `transcript: ${displayWanixReportPath(step.outputPath)}` : undefined,
		step.metadataPath ? `metadata: ${displayWanixReportPath(step.metadataPath)}` : undefined,
	].filter((detail): detail is string => Boolean(detail));
	if (details.length === 0) {
		return [`${index}. ${step.label}`, ""];
	}
	return [
		`${index}. ${step.label}`,
		...details.map((detail) => `   - ${detail}`),
		"",
	];
}

function uniqueReportPaths(paths: string[]): string[] {
	return [...new Set(paths.filter((path) => path.length > 0))];
}

function displayWanixReportPath(path: string): string {
	return absoluteWanixPath(path);
}

async function writeCockpitTourReport(
	fsys: any,
	bridge: WanixBridge,
	startedAt: Date,
	completedAt: Date,
	status: "complete" | "failed",
	steps: CockpitTourStep[],
	error?: unknown,
): Promise<string> {
	await fsys.makeDirAll(parentPath(COCKPIT_TOUR_REPORT_PATH));
	const report = cockpitTourReportMarkdown({
		startedAt,
		completedAt,
		status,
		steps,
		reportPath: COCKPIT_TOUR_REPORT_PATH,
		error: error instanceof Error ? error.message : error ? String(error) : undefined,
	});
	await fsys.writeFile(COCKPIT_TOUR_REPORT_PATH, report);
	bridge.refresh(COCKPIT_TOUR_REPORT_PATH);
	return COCKPIT_TOUR_REPORT_PATH;
}

function cockpitTourReportMarkdown(report: {
	startedAt: Date;
	completedAt: Date;
	status: "complete" | "failed";
	steps: CockpitTourStep[];
	reportPath: string;
	error?: string;
}): string {
	const artifacts = uniqueReportPaths(report.steps.flatMap((step) => step.artifacts));
	return [
		"# Wanix OS Cockpit Tour",
		"",
		`Status: ${report.status}`,
		"Arc: This is an OS. It runs multiple runtimes. Linux can mount it. It can serve apps. Agents can operate it.",
		`Started: ${report.startedAt.toISOString()}`,
		`Completed: ${report.completedAt.toISOString()}`,
		`Report: ${displayWanixReportPath(report.reportPath)}`,
		report.error ? `Error: ${report.error}` : undefined,
		"",
		"## Tour Steps",
		"",
		...report.steps.flatMap((step, index) => cockpitTourStepLines(step, index + 1)),
		"## Key Artifacts",
		"",
		...artifacts.map((path) => `- ${displayWanixReportPath(path)}`),
		"",
		"## What To Inspect Next",
		"",
		"- Expand Tasks to inspect qjs and wasm task rows, transcripts, metadata, and #task service directories.",
		"- Expand Route Runs to inspect HTTP route response reports and task stdout/stderr traces.",
		"- Expand Agent to reopen the repair report, result file, transcript captures, and before/after diff.",
		"- Open Namespace entries for #task and #term to inspect Wanix service files directly.",
		"",
	].filter((line): line is string => line !== undefined).join("\n");
}

function cockpitTourStepLines(step: CockpitTourStep, index: number): string[] {
	return [
		`${index}. ${step.label} (${step.status})`,
		`   - ${step.description}`,
		...step.artifacts.map((path) => `   - ${displayWanixReportPath(path)}`),
		step.error ? `   - error: ${step.error}` : undefined,
		"",
	].filter((line): line is string => line !== undefined);
}

async function refreshWorkbenchFiles(bridge: WanixBridge): Promise<void> {
	bridge.refresh();
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
}

async function refreshWanixPaths(bridge: WanixBridge, paths: string[]): Promise<void> {
	const refreshPaths = new Set<string>();
	for (const path of paths) {
		const normalized = normalizeWanixPath(path);
		refreshPaths.add(normalized);
		refreshPaths.add(parentPath(normalized) || "/");
	}
	for (const path of refreshPaths) {
		bridge.refresh(path);
	}
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

function createQjsShellTerminal(config: Config, onFilesystemActivity?: (activity?: FilesystemActivity) => void, systemView?: WanixSystemView) {
	const writeEmitter = new vscode.EventEmitter<string>();
	const closeEmitter = new vscode.EventEmitter<number | void>();
	const dec = new TextDecoder();
	const enc = new TextEncoder();
	let socket: WebSocket | undefined;
	let opened = false;
	let closed = false;
	let shellTaskId: string | undefined;
	const shellInput = newShellInputState(config);
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
			const activities = trackShellInput(shellInput, data);
			sendInput(enc.encode(data));
			if (activities.length > 0) {
				for (const activity of activities) {
					notifyFilesystemActivity(onFilesystemActivity, activity);
				}
			} else if (data.includes('\r') || data.includes('\n')) {
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

async function createTerminal(fsys: any, config: Config, onFilesystemActivity?: (activity?: FilesystemActivity) => void, options: TerminalOptions = {}) {
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
	const shellInput = newShellInputState(config);
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
				const activities = trackShellInput(shellInput, data);
				writable.write(enc.encode(data));
				if (activities.length > 0) {
					for (const activity of activities) {
						notifyFilesystemActivity(onFilesystemActivity, activity);
					}
				} else if (data.includes('\r') || data.includes('\n')) {
					notifyFilesystemActivity(onFilesystemActivity);
				}
				return;
			}
			// may add line discipline as mode to terminals but for now we
			// do as plan 9 and handle it here in "userspace"
			if (data === '\r') {
				const activity = shellLineActivity(buffer, shellInput);
				writeEmitter.fire('\r\n');           // echo newline
				writable.write(enc.encode(buffer+"\n"));
				buffer = '';
				notifyFilesystemActivity(onFilesystemActivity, activity);
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

type ShellInputState = {
	cwd: string;
	line: string;
};

function newShellInputState(config: Config): ShellInputState {
	return {
		cwd: resolveWanixPath(".", config.shell?.wd || "."),
		line: "",
	};
}

function trackShellInput(state: ShellInputState, data: string): FilesystemActivity[] {
	const activities: FilesystemActivity[] = [];
	for (const char of data) {
		if (char === "\r" || char === "\n") {
			const activity = shellLineActivity(state.line, state);
			if (activity) {
				activities.push(activity);
			}
			state.line = "";
		} else if (char === "\x7f" || char === "\b") {
			state.line = state.line.slice(0, -1);
		} else if (char === "\x03") {
			state.line = "";
		} else if (char >= " ") {
			state.line += char;
		}
	}
	return activities;
}

function shellLineActivity(line: string, state: ShellInputState): FilesystemActivity | undefined {
	const words = shellWords(line.trim());
	if (words.length === 0) {
		return undefined;
	}
	const command = words[0];
	if (command === "cd") {
		state.cwd = resolveWanixPath(state.cwd, words[1] || ".");
		return undefined;
	}
	const redirected = shellRedirectionPaths(words, state.cwd);
	switch (command) {
		case "write":
			return shellPathActivity("write", words.length >= 3 && words[1] ? [resolveWanixPath(state.cwd, words[1])] : redirected);
		case "mkdir":
		case "rm":
		case "rmdir":
			return shellPathActivity(command, words[1] ? [resolveWanixPath(state.cwd, words[1])] : redirected);
		case "cp":
			return shellPathActivity("cp", words[2] ? [resolveWanixPath(state.cwd, words[2])] : redirected);
		case "mv":
			return shellPathActivity("mv", [
				...(words[1] ? [resolveWanixPath(state.cwd, words[1])] : []),
				...(words[2] ? [resolveWanixPath(state.cwd, words[2])] : []),
				...redirected,
			]);
		case "ln":
			return words[1] === "-s"
				? shellPathActivity("ln -s", words[3] ? [resolveWanixPath(state.cwd, words[3])] : redirected)
				: shellPathActivity("ln", redirected);
		default:
			return shellPathActivity("redirect", redirected);
	}
}

function shellPathActivity(command: string, paths: string[]): FilesystemActivity | undefined {
	const unique = [...new Set(paths.filter((path) => path && !path.startsWith("#")))];
	if (unique.length === 0) {
		return undefined;
	}
	const openPath = shellActivityOpenPath(command, unique);
	const label = command === "mv" && unique.length >= 2
		? `shell mv ${displayWanixPath(unique[0])} -> ${displayWanixPath(unique[1])}`
		: `shell ${command} ${displayWanixPath(unique[unique.length - 1])}`;
	return { label, openPath, paths: unique };
}

function shellActivityOpenPath(command: string, paths: string[]): string {
	if (command === "rm" || command === "rmdir") {
		return parentPath(paths[0]) || "/";
	}
	if (command === "mv" && paths.length >= 2) {
		return paths[1];
	}
	return paths[paths.length - 1];
}

function shellRedirectionPaths(words: string[], cwd: string): string[] {
	const paths: string[] = [];
	for (let index = 0; index < words.length; index += 1) {
		const word = words[index];
		if (word === ">" || word === "1>" || word === "2>" || word === ">>" || word === "1>>" || word === "2>>") {
			if (words[index + 1]) {
				paths.push(resolveWanixPath(cwd, words[index + 1]));
			}
		} else if (word.startsWith("2>") && word.length > 2) {
			paths.push(resolveWanixPath(cwd, word.slice(2)));
		} else if (word.startsWith(">") && word.length > 1) {
			paths.push(resolveWanixPath(cwd, word.slice(1)));
		}
	}
	return paths;
}

function shellWords(line: string): string[] {
	const words: string[] = [];
	let current = "";
	let quote: "'" | "\"" | undefined;
	let escaping = false;
	for (const char of line) {
		if (escaping) {
			current += char;
			escaping = false;
			continue;
		}
		if (char === "\\" && quote !== "'") {
			escaping = true;
			continue;
		}
		if (quote) {
			if (char === quote) {
				quote = undefined;
			} else {
				current += char;
			}
			continue;
		}
		if (char === "'" || char === "\"") {
			quote = char;
			continue;
		}
		if (/\s/.test(char)) {
			if (current.length > 0) {
				words.push(current);
				current = "";
			}
			continue;
		}
		current += char;
	}
	if (current.length > 0) {
		words.push(current);
	}
	return words;
}

function resolveWanixPath(cwd: string, path: string): string {
	if (path.startsWith("#")) {
		return path;
	}
	const rooted = path.startsWith("/");
	const prefix = rooted || cwd === "." ? "" : cwd;
	const raw = rooted ? path : (prefix ? `${prefix}/${path}` : path);
	const parts: string[] = [];
	for (const part of raw.split("/")) {
		if (!part || part === ".") {
			continue;
		}
		if (part === "..") {
			parts.pop();
		} else {
			parts.push(part);
		}
	}
	const normalized = parts.join("/");
	return rooted ? `/${normalized}` || "/" : normalized || ".";
}

function displayWanixPath(path: string): string {
	return path === "." ? "/" : path;
}

function notifyFilesystemActivity(callback?: (activity?: FilesystemActivity) => void, activity?: FilesystemActivity): void {
	if (!callback) {
		return;
	}
	delay(250).then(() => callback(activity)).catch((error) => {
		console.warn("Wanix filesystem refresh failed", error);
	});
	delay(1000).then(() => callback(activity)).catch((error) => {
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

function normalizeWanixPath(path: string): string {
	return resolveWanixPath(".", path);
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
