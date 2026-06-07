
import * as vscode from 'vscode';
import { openAgentToolContract, writeAgentToolContract } from './agent-tool-contract.js';
import { AGENT_BROKEN_PATH, installAgentRepairDemo, repairQjsProgram } from './agent-repair-demo.js';
import { WanixBridge, type WanixBridgeMutation } from './bridge.js';
import { COCKPIT_SELF_CHECK_JSON_PATH, COCKPIT_SELF_CHECK_MD_PATH, COCKPIT_SELF_CHECK_PROBE_PATH, runCockpitSelfCheck } from './cockpit-self-check.js';
import { DUET_DEMO_STEPS, DUET_OUTPUT_PATH, installDuetDemo, resetDuetDemo } from './duet-demo.js';
import { copyHttpAppUrl, createHttpApp, installHttpAppDemo, openHttpAppCatalog, openHttpAppDemo, openHttpAppHandler, openHttpCounterDemo, openHttpWasmDemo, previewHttpAppPath, previewHttpCatalogApp, publishHttpAppDataStores, publishHttpAppsToSystemView, type HttpAppCatalogTarget, type HttpAppRouteConfig } from './http-app-demo.js';
import { createQjsStarter } from './qjs-starter.js';
import { WANIX_INSPECT_SCHEME, WanixServiceInspector } from './service-inspector.js';
import { WanixSystemView, type WanixServiceTask, type WanixServiceTerminal, type WanixShellArchiveRecord } from './system-view.js';
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
	description?: string;
	evidence?: string;
	openPath?: string;
	paths?: string[];
};

type ShellHistoryEntry = {
	schema?: string;
	observedAtUnixMillis?: number;
	taskId?: string;
	terminalId?: string;
	cwd?: string;
	command?: string;
	outcome?: {
		status?: string;
		changed?: boolean;
		evidence?: string;
		diagnostic?: string;
		exitCode?: number;
		terminalOutput?: string;
	};
	operation?: {
		kind?: string;
		status?: string;
		source?: string;
		target?: string;
		paths?: string[];
	};
};

type ShellHistoryPick = vscode.QuickPickItem & {
	entry: ShellHistoryEntry;
};

type ShellHistoryCompactPick = vscode.QuickPickItem & {
	retentionKind: "count" | "age";
	keepCount?: number;
	ageMs?: number;
};

type ShellHistoryArchivePick = vscode.QuickPickItem & {
	archiveDir: string;
	commandsPath: string;
};

type ShellHistoryArchiveTarget = {
	archiveDir?: string;
	commandsPath?: string;
	bundleJsonPath?: string;
};

type ShellHistoryArchiveInfo = {
	name: string;
	archiveDir: string;
	commandsPath: string;
	indexPath: string;
	manifestPath: string;
	summaryPath: string;
	latestMarkdownPath: string;
	generatedAt: string;
	generatedAtUnixMillis?: number;
	commandCount: number;
	firstObservedAt?: string;
	lastObservedAt?: string;
	compareMarkdownPath: string;
	compareJsonPath: string;
	compareGeneratedAt?: string;
	archivedOnlyCount?: number;
	liveOnlyCount?: number;
	wasLastRestored?: boolean;
	bundleMarkdownPath: string;
	bundleJsonPath: string;
	bundleGeneratedAt?: string;
	bundleFileCount?: number;
	importMarkdownPath: string;
	importJsonPath: string;
	importGeneratedAt?: string;
};

type ShellHistoryArchiveInventory = {
	generatedAt: Date;
	archives: ShellHistoryArchiveInfo[];
	lastRestoredArchiveDir?: string;
};

type ShellHistoryArchiveBundleFile = {
	path: string;
	bytes: number;
	content: string;
};

type ShellHistoryArchiveBundle = {
	archive: ShellHistoryArchiveInfo;
	files: ShellHistoryArchiveBundleFile[];
	generatedAt: Date;
};

type ShellHistoryArchiveBundlePick = vscode.QuickPickItem & {
	archive: ShellHistoryArchiveInfo;
};

type ShellHistoryArchiveBundleImportPick = vscode.QuickPickItem & {
	bundleText: string;
	sourcePath?: string;
};

type ShellHistoryArchivePrunePick = vscode.QuickPickItem & {
	retentionKind: "count" | "age" | "selected";
	keepCount?: number;
	ageMs?: number;
};

type ShellHistoryArtifact = {
	entry: ShellHistoryEntry;
	index: number;
	path: string;
};

type ShellHistoryArtifactLinks = {
	summaryPath: string;
	latestPath: string;
};

type ShellHistoryLatestPaths = {
	historyPath?: string;
	latestJsonPath?: string;
	latestMarkdownPath?: string;
	note?: string;
};

type ShellHistoryRewriteOptions = {
	latestNote?: string;
	removeSelected?: boolean;
};

type ShellHistorySummaryPaths = {
	archiveIndexPath?: string;
	archiveManifestPath?: string;
	commandDir?: string;
	historyPath?: string;
	latestPath?: string;
};

type ShellHistoryArchive = {
	archiveDir: string;
	artifacts: ShellHistoryArtifact[];
	commandsPath: string;
	generatedAt: Date;
	indexPath: string;
	latestJsonPath: string;
	latestMarkdownPath: string;
	manifestPath: string;
	summaryPath: string;
};

type ShellHistoryComparedEntry = {
	entry: ShellHistoryEntry;
	index: number;
	artifact?: ShellHistoryArtifact;
};

type ShellHistoryArchiveComparison = {
	archiveDir: string;
	archiveEntries: ShellHistoryEntry[];
	liveEntries: ShellHistoryEntry[];
	generatedAt: Date;
	retained: ShellHistoryComparedEntry[];
	archivedOnly: ShellHistoryComparedEntry[];
	liveOnly: ShellHistoryComparedEntry[];
};

const TASK_RUNNERS: Record<TaskRunKind, { extension: string; label: string }> = {
	qjs: { extension: ".js", label: "JavaScript" },
	wasm: { extension: ".wasm", label: "WASM" },
};
const TASK_OUTPUT_DIR = ".wanix/tasks";
const TASK_OUTPUT_MAX_CHARS = 512 * 1024;
const COCKPIT_TOUR_REPORT_PATH = ".wanix/cockpit-tour.md";
const COCKPIT_REPORT_INDEX_MD_PATH = ".wanix/cockpit-reports.md";
const COCKPIT_REPORT_INDEX_JSON_PATH = ".wanix/cockpit-reports.json";
const DATA_STORE_INDEX_MD_PATH = ".wanix/data-stores.md";
const DATA_STORE_INDEX_JSON_PATH = ".wanix/data-stores.json";
const SHELL_HISTORY_JSONL_PATH = ".wanix/qjs-shell/commands.jsonl";
const SHELL_HISTORY_JSON_PATH = ".wanix/qjs-shell/latest.json";
const SHELL_HISTORY_MD_PATH = ".wanix/qjs-shell/latest.md";
const SHELL_HISTORY_SELECTED_MD_PATH = ".wanix/qjs-shell/selected.md";
const SHELL_HISTORY_SUMMARY_MD_PATH = ".wanix/qjs-shell/summary.md";
const SHELL_HISTORY_RESTORE_MD_PATH = ".wanix/qjs-shell/restored.md";
const SHELL_HISTORY_RESTORE_JSON_PATH = ".wanix/qjs-shell/restored.json";
const SHELL_HISTORY_COMMANDS_DIR = ".wanix/qjs-shell/commands";
const SHELL_HISTORY_ARCHIVE_DIR = ".wanix/qjs-shell/archive";
const SHELL_HISTORY_ARCHIVE_INVENTORY_MD_PATH = ".wanix/qjs-shell/archive/inventory.md";
const SHELL_HISTORY_ARCHIVE_INVENTORY_JSON_PATH = ".wanix/qjs-shell/archive/inventory.json";
const SHELL_HISTORY_ARCHIVE_PRUNE_MD_PATH = ".wanix/qjs-shell/archive/pruned.md";
const SHELL_HISTORY_ARCHIVE_PRUNE_JSON_PATH = ".wanix/qjs-shell/archive/pruned.json";
const SHELL_HISTORY_ARCHIVE_BUNDLE_MD_NAME = "bundle.md";
const SHELL_HISTORY_ARCHIVE_BUNDLE_JSON_NAME = "bundle.json";
const SHELL_HISTORY_ARCHIVE_IMPORT_MD_NAME = "imported.md";
const SHELL_HISTORY_ARCHIVE_IMPORT_JSON_NAME = "imported.json";
const SHELL_HISTORY_COMPARE_MD_NAME = "compare-live.md";
const SHELL_HISTORY_COMPARE_JSON_NAME = "compare-live.json";
const SHELL_HISTORY_LATEST_MARKDOWN_LIMIT = 12;
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
				description: activity.description,
				evidence: activity.evidence,
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
			description: activity.description,
			evidence: activity.evidence,
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
		void hydrateShellArchiveInventory(fsys, bridge, systemView).catch((error) => {
			console.warn("failed to hydrate shell archive inventory", error);
		});
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
					const shellCwd = config.shell?.wd || ".";
					systemView.filesystemActivity(`shell opened in ${displayWanixPath(shellCwd)}`, { path: shellCwd });
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
		context.subscriptions.push(vscode.commands.registerCommand('workbench.runCockpitSelfCheck', async () => {
			try {
				await runCockpitSelfCheck(fsys, bridge, config, systemView);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.prepareCockpitReports', async () => {
			try {
				await prepareCockpitReports(fsys, bridge, config, systemView);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openCockpitReports', async () => {
			try {
				await publishCockpitReportInventory(fsys, bridge, systemView);
				await openWanixPath(COCKPIT_REPORT_INDEX_MD_PATH);
				revealWanixSystemView();
				vscode.window.showInformationMessage(`Opened Wanix cockpit report inventory`);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openDataStoreInventory', async () => {
			try {
				await publishDataStoreInventory(fsys, bridge, systemView);
				await openWanixPath(DATA_STORE_INDEX_MD_PATH);
				revealWanixSystemView();
				vscode.window.showInformationMessage(`Opened Wanix data store index`);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openShellCommandHistory', async () => {
			try {
				await openShellCommandHistory(fsys, bridge, systemView);
				revealWanixSystemView();
				vscode.window.showInformationMessage(`Opened qjs shell command history`);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.searchShellCommandHistory', async () => {
			try {
				await searchShellCommandHistory(fsys, bridge, systemView);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openShellHistorySummary', async () => {
			try {
				await openShellHistorySummary(fsys, bridge, systemView);
				revealWanixSystemView();
				vscode.window.showInformationMessage(`Opened qjs shell history summary`);
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.archiveShellCommandHistory', async () => {
			try {
				await archiveShellCommandHistory(fsys, bridge, systemView);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openShellHistoryArchiveInventory', async () => {
			try {
				await openShellHistoryArchiveInventory(fsys, bridge, systemView);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.exportShellHistoryArchiveBundle', async (target?: ShellHistoryArchiveTarget) => {
			try {
				await exportShellHistoryArchiveBundle(fsys, bridge, systemView, target);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.importShellHistoryArchiveBundle', async (target?: ShellHistoryArchiveTarget) => {
			try {
				await importShellHistoryArchiveBundle(fsys, bridge, systemView, target);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.compareShellHistoryArchive', async (target?: ShellHistoryArchiveTarget) => {
			try {
				await compareShellHistoryArchive(fsys, bridge, systemView, target);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.restoreShellHistoryArchive', async (target?: ShellHistoryArchiveTarget) => {
			try {
				await restoreShellHistoryArchive(fsys, bridge, systemView, target);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.pruneShellHistoryArchives', async (target?: ShellHistoryArchiveTarget) => {
			try {
				await pruneShellHistoryArchives(fsys, bridge, systemView, target);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.compactShellCommandHistory', async () => {
			try {
				await compactShellCommandHistory(fsys, bridge, systemView);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.clearShellCommandHistory', async () => {
			try {
				await clearShellCommandHistory(fsys, bridge, systemView);
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
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openAgentToolContract', async () => {
			try {
				await openAgentToolContract(fsys, bridge, systemView);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.clearFinishedSystemRows', () => {
			systemView.clearFinishedRows();
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openSystemJournal', async () => {
			try {
				await openSystemJournal(fsys, bridge, config, systemView);
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
		context.subscriptions.push(vscode.commands.registerCommand('workbench.newHttpApp', async () => {
			try {
				await createHttpApp(fsys, bridge, config, systemView);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.previewCurrentHttpApp', async (resource?: vscode.Uri) => {
			try {
				const uri = commandWanixUri(resource) || currentWanixEditor()?.document.uri;
				if (!uri) {
					throw new Error("Open an /apps HTTP handler before previewing the current app");
				}
				if (uri.scheme !== WanixBridge.scheme) {
					throw new Error("Preview Current HTTP App expects a wanix: file");
				}
				await saveOpenDocument(uri, "HTTP app");
				await previewHttpAppPath(fsys, bridge, config, systemView, bridge.normalizePath(uri.path));
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openHttpAppCatalog', async () => {
			try {
				await openHttpAppCatalog(fsys, bridge, config, systemView);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.previewHttpCatalogApp', async (target?: HttpAppCatalogTarget) => {
			try {
				await previewHttpCatalogApp(fsys, bridge, config, systemView, target);
				revealWanixSystemView();
			} catch (error) {
				vscode.window.showErrorMessage(error instanceof Error ? error.message : String(error));
			}
		}));
		context.subscriptions.push(vscode.commands.registerCommand('workbench.openHttpAppSource', async (target?: string | { sourcePath?: string }) => {
			try {
				const sourcePath = taskSourcePath(target);
				if (!sourcePath) {
					throw new Error("HTTP app source is not available");
				}
				await openWanixPath(sourcePath);
				systemView.filesystemActivity(`opened http app source ${baseName(sourcePath)}`);
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

async function openSystemJournal(fsys: any, bridge: WanixBridge, config: Config, systemView: WanixSystemView): Promise<void> {
	await writeSystemJournal(fsys, bridge, config, systemView);
	await openWanixPath(SYSTEM_JOURNAL_PATH);
	vscode.window.showInformationMessage("Opened Wanix system journal");
}

async function writeSystemJournal(
	fsys: any,
	bridge: WanixBridge,
	config: Config,
	systemView: WanixSystemView,
): Promise<{ journalPath: string; statePath: string }> {
	await fsys.makeDirAll(".wanix");
	const generatedAt = new Date();
	await publishHttpAppsToSystemView(fsys, config, systemView);
	await publishHttpAppDataStores(fsys, systemView);
	systemView.filesystemActivity("system state written", { path: SYSTEM_STATE_PATH });
	systemView.filesystemActivity("system journal written", { path: SYSTEM_JOURNAL_PATH });
	systemView.reportPublished("System Journal", SYSTEM_JOURNAL_PATH, {
		kind: "state",
		description: "live state snapshot",
		icon: "notebook",
		artifacts: [SYSTEM_JOURNAL_PATH, SYSTEM_STATE_PATH],
	});
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
	return {
		journalPath: SYSTEM_JOURNAL_PATH,
		statePath: SYSTEM_STATE_PATH,
	};
}

async function prepareCockpitReports(
	fsys: any,
	bridge: WanixBridge,
	config: Config,
	systemView: WanixSystemView,
): Promise<void> {
	systemView.filesystemActivity("cockpit preparation started");
	await writeAgentToolContract(fsys, bridge, systemView);
	await writeSystemJournal(fsys, bridge, config, systemView);
	await runCockpitSelfCheck(fsys, bridge, config, systemView);
	await publishCockpitReportInventory(fsys, bridge, systemView);
	await writeSystemJournal(fsys, bridge, config, systemView);
	await publishCockpitReportInventory(fsys, bridge, systemView);
	systemView.filesystemActivity("cockpit reports prepared", {
		path: COCKPIT_REPORT_INDEX_MD_PATH,
		paths: [COCKPIT_REPORT_INDEX_MD_PATH, COCKPIT_REPORT_INDEX_JSON_PATH],
	});
	await openWanixPath(COCKPIT_REPORT_INDEX_MD_PATH);
	vscode.window.showInformationMessage(`Wanix cockpit reports prepared in ${COCKPIT_REPORT_INDEX_MD_PATH}`);
}

async function publishCockpitReportInventory(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	const generatedAt = new Date();
	await fsys.makeDirAll(".wanix");
	await publishShellCommandHistoryReport(fsys, systemView);
	systemView.reportPublished("Cockpit Report Index", COCKPIT_REPORT_INDEX_MD_PATH, {
		kind: "manifest",
		description: "live report inventory",
		icon: "notebook",
		artifacts: [COCKPIT_REPORT_INDEX_MD_PATH, COCKPIT_REPORT_INDEX_JSON_PATH],
	});
	await fsys.writeFile(COCKPIT_REPORT_INDEX_JSON_PATH, systemView.reportInventoryJson({
		generatedAt,
		markdownPath: COCKPIT_REPORT_INDEX_MD_PATH,
		jsonPath: COCKPIT_REPORT_INDEX_JSON_PATH,
	}));
	await fsys.writeFile(COCKPIT_REPORT_INDEX_MD_PATH, systemView.reportInventoryMarkdown({
		generatedAt,
		markdownPath: COCKPIT_REPORT_INDEX_MD_PATH,
		jsonPath: COCKPIT_REPORT_INDEX_JSON_PATH,
	}));
	await refreshWanixPaths(bridge, [COCKPIT_REPORT_INDEX_MD_PATH, COCKPIT_REPORT_INDEX_JSON_PATH]);
}

async function openShellCommandHistory(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	const paths = await existingShellHistoryPaths(fsys);
	if (paths.length === 0) {
		throw new Error("No qjs shell command history yet. Run a served shell command first.");
	}
	const openPath = paths.includes(SHELL_HISTORY_MD_PATH) ? SHELL_HISTORY_MD_PATH : paths[0];
	publishShellCommandHistoryReportFromPaths(systemView, paths, openPath);
	systemView.filesystemActivity("shell command history opened", { path: openPath, paths });
	await refreshWanixPaths(bridge, paths);
	await openWanixPath(openPath);
}

async function searchShellCommandHistory(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	const entries = await readShellHistoryEntries(fsys);
	if (entries.length === 0) {
		throw new Error("No qjs shell command history entries yet. Run a served shell command first.");
	}
	const picks = shellHistoryPicks(entries);
	const pick = await vscode.window.showQuickPick(picks, {
		placeHolder: "Search qjs shell command history",
		matchOnDescription: true,
		matchOnDetail: true,
	});
	if (!pick) {
		return;
	}
	await fsys.makeDirAll(parentPath(SHELL_HISTORY_SELECTED_MD_PATH));
	await fsys.writeFile(SHELL_HISTORY_SELECTED_MD_PATH, shellHistorySelectionMarkdown(pick.entry));
	const paths = await existingShellHistoryPaths(fsys);
	const reportPaths = paths.includes(SHELL_HISTORY_SELECTED_MD_PATH) ? paths : [SHELL_HISTORY_SELECTED_MD_PATH, ...paths];
	const reportOpenPath = reportPaths.includes(SHELL_HISTORY_MD_PATH) ? SHELL_HISTORY_MD_PATH : SHELL_HISTORY_SELECTED_MD_PATH;
	publishShellCommandHistoryReportFromPaths(systemView, reportPaths, reportOpenPath);
	systemView.filesystemActivity("shell history selection opened", {
		description: shellHistoryStatus(pick.entry),
		path: SHELL_HISTORY_SELECTED_MD_PATH,
		paths: reportPaths,
	});
	await refreshWanixPaths(bridge, reportPaths);
	await openWanixPath(SHELL_HISTORY_SELECTED_MD_PATH);
}

async function openShellHistorySummary(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	const entries = await readShellHistoryEntries(fsys);
	if (entries.length === 0) {
		throw new Error("No qjs shell command history entries yet. Run a served shell command first.");
	}
	await fsys.makeDirAll(parentPath(SHELL_HISTORY_SUMMARY_MD_PATH));
	const artifacts = shellHistoryArtifacts(entries);
	await writeShellHistoryCommandArtifacts(fsys, artifacts);
	await fsys.writeFile(SHELL_HISTORY_SUMMARY_MD_PATH, shellHistorySummaryMarkdown(artifacts, new Date()));
	const paths = await existingShellHistoryPaths(fsys);
	const reportPaths = paths.includes(SHELL_HISTORY_SUMMARY_MD_PATH) ? paths : [SHELL_HISTORY_SUMMARY_MD_PATH, ...paths];
	publishShellCommandHistoryReportFromPaths(systemView, reportPaths, shellHistoryReportOpenPath(reportPaths));
	systemView.filesystemActivity("shell history summary opened", {
		description: `${entries.length} commands grouped`,
		path: SHELL_HISTORY_SUMMARY_MD_PATH,
		paths: reportPaths,
	});
	await refreshWanixPaths(bridge, reportPaths);
	await openWanixPath(SHELL_HISTORY_SUMMARY_MD_PATH);
}

async function archiveShellCommandHistory(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	const entries = await readShellHistoryEntries(fsys);
	if (entries.length === 0) {
		throw new Error("No qjs shell command history entries yet. Run a served shell command first.");
	}
	const generatedAt = new Date();
	const archiveDir = `${SHELL_HISTORY_ARCHIVE_DIR}/${shellHistoryArchiveId(generatedAt)}`;
	const paths = await writeShellHistoryArchiveArtifacts(fsys, entries, archiveDir, generatedAt);
	const indexPath = `${archiveDir}/index.md`;
	const inventory = await shellHistoryArchiveInventory(fsys, generatedAt);
	const inventoryPaths = await writeShellHistoryArchiveInventory(fsys, inventory);
	publishShellArchiveInventoryToSystemView(systemView, inventory.archives);
	const reportPaths = [...paths, ...inventoryPaths];
	publishShellCommandHistoryArchiveReport(systemView, archiveDir, reportPaths);
	systemView.filesystemActivity("shell command history archived", {
		description: `${entries.length} commands exported`,
		path: indexPath,
		paths: reportPaths,
	});
	await refreshWanixPaths(bridge, [SHELL_HISTORY_ARCHIVE_DIR, archiveDir, ...reportPaths]);
	await openWanixPath(indexPath);
	vscode.window.showInformationMessage(`Archived qjs shell history: ${entries.length} commands`);
}

async function openShellHistoryArchiveInventory(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	const inventory = await shellHistoryArchiveInventory(fsys, new Date());
	const paths = await writeShellHistoryArchiveInventory(fsys, inventory);
	publishShellArchiveInventoryToSystemView(systemView, inventory.archives);
	publishShellHistoryArchiveInventoryReport(systemView, paths);
	systemView.filesystemActivity("shell archive inventory published", {
		description: `${inventory.archives.length} archives`,
		path: SHELL_HISTORY_ARCHIVE_INVENTORY_MD_PATH,
		paths,
	});
	await refreshWanixPaths(bridge, [SHELL_HISTORY_ARCHIVE_DIR, ...paths]);
	await openWanixPath(SHELL_HISTORY_ARCHIVE_INVENTORY_MD_PATH);
	vscode.window.showInformationMessage(`Opened qjs shell archive inventory: ${inventory.archives.length} archives`);
}

async function hydrateShellArchiveInventory(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	const persisted = await shellHistoryPersistedArchiveInfos(fsys);
	const scanned = await shellHistoryArchiveInfos(fsys);
	const archives = scanned.length > 0 ? scanned : persisted;
	if (archives.length === 0) {
		return;
	}
	const generatedAt = new Date();
	const inventory = await shellHistoryArchiveInventory(fsys, generatedAt, archives);
	publishShellArchiveInventoryToSystemView(systemView, inventory.archives);
	if (scanned.length > 0) {
		const paths = await writeShellHistoryArchiveInventory(fsys, inventory);
		await refreshWanixPaths(bridge, [SHELL_HISTORY_ARCHIVE_DIR, ...paths]);
	}
}

async function exportShellHistoryArchiveBundle(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
	target?: ShellHistoryArchiveTarget,
): Promise<void> {
	const targetedArchive = await shellHistoryArchiveInfoForTarget(fsys, target);
	const archive = targetedArchive || await pickShellHistoryArchiveBundle(fsys);
	if (!archive) {
		return;
	}
	const generatedAt = new Date();
	const bundle = await shellHistoryArchiveBundle(fsys, archive, generatedAt);
	const paths = await writeShellHistoryArchiveBundle(fsys, bundle);
	const inventory = await shellHistoryArchiveInventory(fsys, generatedAt);
	const inventoryPaths = await writeShellHistoryArchiveInventory(fsys, inventory);
	publishShellArchiveInventoryToSystemView(systemView, inventory.archives);
	const reportPaths = [...paths, ...inventoryPaths];
	publishShellHistoryArchiveBundleReport(systemView, archive, reportPaths);
	systemView.filesystemActivity("shell archive bundle exported", {
		description: `${bundle.files.length} files, ${shellHistoryBytesLabel(shellHistoryBundleBytes(bundle.files))}`,
		path: archive.bundleMarkdownPath,
		paths: reportPaths,
	});
	await refreshWanixPaths(bridge, [archive.archiveDir, SHELL_HISTORY_ARCHIVE_DIR, ...reportPaths]);
	await openWanixPath(archive.bundleMarkdownPath);
	vscode.window.showInformationMessage(`Exported qjs shell archive bundle: ${archive.name}, ${bundle.files.length} files`);
}

async function importShellHistoryArchiveBundle(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
	target?: ShellHistoryArchiveTarget,
): Promise<void> {
	const pick = await shellHistoryArchiveBundleImportPick(fsys, target);
	if (!pick) {
		return;
	}
	const parsed = shellHistoryParseArchiveBundle(pick.bundleText);
	const generatedAt = new Date();
	const paths = await rehydrateShellHistoryArchiveBundle(fsys, parsed, pick.sourcePath, generatedAt);
	const inventory = await shellHistoryArchiveInventory(fsys, generatedAt);
	const inventoryPaths = await writeShellHistoryArchiveInventory(fsys, inventory);
	publishShellArchiveInventoryToSystemView(systemView, inventory.archives);
	const reportPaths = [...paths, ...inventoryPaths];
	publishShellHistoryArchiveImportReport(systemView, parsed.archive, reportPaths);
	systemView.filesystemActivity("shell archive bundle imported", {
		description: `${parsed.files.length} files rehydrated`,
		path: parsed.archive.importMarkdownPath,
		paths: reportPaths,
	});
	await refreshWanixPaths(bridge, [parsed.archive.archiveDir, SHELL_HISTORY_ARCHIVE_DIR, ...reportPaths]);
	await openWanixPath(parsed.archive.importMarkdownPath);
	vscode.window.showInformationMessage(`Imported qjs shell archive bundle: ${parsed.archive.name}, ${parsed.files.length} files`);
}

async function compareShellHistoryArchive(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
	target?: ShellHistoryArchiveTarget,
): Promise<void> {
	const pick = await pickShellHistoryArchive(fsys, target, "Compare qjs shell history archive with current live history");
	if (!pick) {
		return;
	}
	const archiveEntries = await readShellHistoryEntries(fsys, pick.commandsPath);
	const liveEntries = await readShellHistoryEntriesOptional(fsys, SHELL_HISTORY_JSONL_PATH);
	const generatedAt = new Date();
	const comparison = shellHistoryArchiveComparison(pick.archiveDir, archiveEntries, liveEntries, generatedAt);
	const paths = await writeShellHistoryArchiveComparison(fsys, comparison);
	const inventory = await shellHistoryArchiveInventory(fsys, generatedAt);
	const inventoryPaths = await writeShellHistoryArchiveInventory(fsys, inventory);
	publishShellArchiveInventoryToSystemView(systemView, inventory.archives);
	const comparePath = `${pick.archiveDir}/${SHELL_HISTORY_COMPARE_MD_NAME}`;
	const reportPaths = [...paths, ...inventoryPaths];
	publishShellCommandHistoryArchiveCompareReport(systemView, pick.archiveDir, reportPaths);
	systemView.filesystemActivity("shell history archive compared", {
		description: `${comparison.archivedOnly.length} archived-only, ${comparison.liveOnly.length} live-only`,
		path: comparePath,
		paths: reportPaths,
	});
	await refreshWanixPaths(bridge, [pick.archiveDir, SHELL_HISTORY_ARCHIVE_DIR, ...reportPaths]);
	await openWanixPath(comparePath);
	vscode.window.showInformationMessage(`Compared qjs shell archive: ${comparison.archivedOnly.length} archived-only, ${comparison.liveOnly.length} live-only`);
}

async function restoreShellHistoryArchive(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
	target?: ShellHistoryArchiveTarget,
): Promise<void> {
	const pick = await pickShellHistoryArchive(fsys, target, "Restore qjs shell history archive into the live history view");
	if (!pick) {
		return;
	}
	const archiveEntries = await readShellHistoryEntries(fsys, pick.commandsPath);
	if (archiveEntries.length === 0) {
		throw new Error(`Archive ${shellHistoryAbsolutePath(pick.archiveDir)} has no command history entries.`);
	}
	const liveEntries = await readShellHistoryEntriesOptional(fsys, SHELL_HISTORY_JSONL_PATH);
	const choice = await vscode.window.showWarningMessage(
		`Restore ${archiveEntries.length} archived qjs shell commands into the live history view? This replaces current live qjs-shell history artifacts but keeps archives intact.`,
		{ modal: true },
		"Restore History",
	);
	if (choice !== "Restore History") {
		return;
	}
	const generatedAt = new Date();
	const paths = await rewriteShellHistoryArtifacts(fsys, archiveEntries, generatedAt, {
		latestNote: `This file was restored from archive ${shellHistoryAbsolutePath(pick.archiveDir)}. The live JSONL now contains ${archiveEntries.length} archived commands; grouped summary and command evidence were regenerated for the live qjs-shell surface.`,
	});
	await fsys.writeFile(SHELL_HISTORY_RESTORE_JSON_PATH, shellHistoryRestoreJson(pick.archiveDir, archiveEntries, liveEntries, generatedAt));
	await fsys.writeFile(SHELL_HISTORY_RESTORE_MD_PATH, shellHistoryRestoreMarkdown(pick.archiveDir, archiveEntries, liveEntries, generatedAt));
	const inventory = await shellHistoryArchiveInventory(fsys, generatedAt);
	const inventoryPaths = await writeShellHistoryArchiveInventory(fsys, inventory);
	publishShellArchiveInventoryToSystemView(systemView, inventory.archives);
	const reportPaths = [SHELL_HISTORY_RESTORE_MD_PATH, SHELL_HISTORY_RESTORE_JSON_PATH, ...paths, ...inventoryPaths];
	publishShellCommandHistoryRestoreReport(systemView, reportPaths);
	systemView.filesystemActivity("shell history archive restored", {
		description: `${archiveEntries.length} commands restored`,
		path: SHELL_HISTORY_RESTORE_MD_PATH,
		paths: reportPaths,
	});
	await refreshWanixPaths(bridge, [".wanix/qjs-shell", SHELL_HISTORY_ARCHIVE_DIR, ...reportPaths]);
	await openWanixPath(SHELL_HISTORY_RESTORE_MD_PATH);
	vscode.window.showInformationMessage(`Restored qjs shell history archive: ${archiveEntries.length} commands`);
}

async function pruneShellHistoryArchives(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
	target?: ShellHistoryArchiveTarget,
): Promise<void> {
	const archives = await shellHistoryArchiveInfos(fsys);
	if (archives.length === 0) {
		throw new Error("No qjs shell history archives yet. Use Archive Shell Command History first.");
	}
	const now = Date.now();
	const targetedArchive = await shellHistoryArchiveInfoForTarget(fsys, target, archives);
	const pick = targetedArchive
		? shellHistorySingleArchivePrunePick(targetedArchive)
		: await vscode.window.showQuickPick(shellHistoryArchivePrunePicks(archives, now), {
			placeHolder: "Prune qjs shell history archives",
			matchOnDescription: true,
			matchOnDetail: true,
		});
	if (!pick) {
		return;
	}
	const removed = targetedArchive ? [targetedArchive] : shellHistoryArchivesToPrune(archives, pick, now);
	if (removed.length === 0) {
		await openShellHistoryArchiveInventory(fsys, bridge, systemView);
		vscode.window.showInformationMessage(`No qjs shell archives matched ${pick.label}; refreshed archive inventory`);
		return;
	}
	const choice = await vscode.window.showWarningMessage(
		`Prune ${removed.length} qjs shell history archive directories using "${pick.label}"? This deletes archived evidence directories but leaves current live shell history untouched.`,
		{ modal: true },
		"Prune Archives",
	);
	if (choice !== "Prune Archives") {
		return;
	}
	for (const archive of removed) {
		await removeShellHistoryArchiveDir(fsys, archive.archiveDir);
	}
	const generatedAt = new Date();
	const remaining = await shellHistoryArchiveInfos(fsys);
	const inventory = await shellHistoryArchiveInventory(fsys, generatedAt, remaining);
	const inventoryPaths = await writeShellHistoryArchiveInventory(fsys, inventory);
	publishShellArchiveInventoryToSystemView(systemView, inventory.archives);
	const prunePaths = await writeShellHistoryArchivePruneReport(fsys, generatedAt, pick, removed, remaining);
	const paths = [...prunePaths, ...inventoryPaths];
	publishShellHistoryArchivePruneReport(systemView, paths);
	systemView.filesystemActivity("shell archives pruned", {
		description: `${removed.length} removed, ${remaining.length} kept`,
		path: SHELL_HISTORY_ARCHIVE_PRUNE_MD_PATH,
		paths,
	});
	await refreshWanixPaths(bridge, [SHELL_HISTORY_ARCHIVE_DIR, ...removed.map((archive) => archive.archiveDir), ...paths]);
	await openWanixPath(SHELL_HISTORY_ARCHIVE_PRUNE_MD_PATH);
	vscode.window.showInformationMessage(`Pruned qjs shell archives: ${removed.length} removed, ${remaining.length} kept`);
}

async function compactShellCommandHistory(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	const entries = await readShellHistoryEntries(fsys);
	if (entries.length === 0) {
		throw new Error("No qjs shell command history entries yet. Run a served shell command first.");
	}
	const now = Date.now();
	const pick = await vscode.window.showQuickPick(shellHistoryCompactPicks(entries, now), {
		placeHolder: "Compact qjs shell command history",
		matchOnDescription: true,
		matchOnDetail: true,
	});
	if (!pick) {
		return;
	}
	const retained = compactShellHistoryEntries(entries, pick, now);
	const generatedAt = new Date();
	if (retained.length < entries.length) {
		const choice = await vscode.window.showWarningMessage(
			`Compact qjs shell command history from ${entries.length} to ${retained.length} commands using "${pick.label}"? This rewrites the JSONL log and generated history artifacts.`,
			{ modal: true },
			"Compact History",
		);
		if (choice !== "Compact History") {
			return;
		}
	}
	const paths = await rewriteShellHistoryArtifacts(fsys, retained, generatedAt, {
		latestNote: `This file was regenerated by browser-side history compaction. The JSONL file keeps ${retained.length} retained commands; the grouped summary links each retained command to its evidence file.`,
	});
	const description = retained.length < entries.length
		? `${entries.length} -> ${retained.length} commands`
		: `${entries.length} commands already fit`;
	publishShellCommandHistoryReportFromPaths(systemView, paths, SHELL_HISTORY_SUMMARY_MD_PATH);
	systemView.filesystemActivity(retained.length < entries.length ? "shell command history compacted" : "shell command history refreshed", {
		description,
		path: SHELL_HISTORY_SUMMARY_MD_PATH,
		paths,
	});
	await refreshWanixPaths(bridge, [".wanix/qjs-shell", SHELL_HISTORY_SELECTED_MD_PATH, ...paths]);
	await openWanixPath(SHELL_HISTORY_SUMMARY_MD_PATH);
	vscode.window.showInformationMessage(retained.length < entries.length
		? `Compacted qjs shell history: ${description}`
		: `qjs shell history already fits ${pick.label}; refreshed history artifacts`);
}

async function clearShellCommandHistory(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	const choice = await vscode.window.showWarningMessage(
		"Clear generated qjs shell command history artifacts?",
		{ modal: true },
		"Clear History",
	);
	if (choice !== "Clear History") {
		return;
	}
	const paths = [
		SHELL_HISTORY_COMMANDS_DIR,
		SHELL_HISTORY_SUMMARY_MD_PATH,
		SHELL_HISTORY_RESTORE_MD_PATH,
		SHELL_HISTORY_RESTORE_JSON_PATH,
		SHELL_HISTORY_SELECTED_MD_PATH,
		SHELL_HISTORY_MD_PATH,
		SHELL_HISTORY_JSON_PATH,
		SHELL_HISTORY_JSONL_PATH,
	];
	for (const path of paths) {
		try {
			if (path === SHELL_HISTORY_COMMANDS_DIR) {
				await removeShellHistoryCommandArtifacts(fsys);
			} else {
				await fsys.remove(path);
			}
		} catch {
			// Missing history artifacts are harmless during a clear operation.
		}
	}
	systemView.filesystemActivity("shell command history cleared", {
		path: ".wanix/qjs-shell",
		paths,
	});
	await refreshWanixPaths(bridge, [".wanix/qjs-shell", ...paths]);
	vscode.window.showInformationMessage("Cleared qjs shell command history");
}

async function readShellHistoryEntries(fsys: any, path = SHELL_HISTORY_JSONL_PATH): Promise<ShellHistoryEntry[]> {
	let text: string;
	try {
		text = await fsys.readText(path);
	} catch {
		if (path === SHELL_HISTORY_JSONL_PATH) {
			throw new Error("No qjs shell command history yet. Run a served shell command first.");
		}
		throw new Error(`No qjs shell command history at ${shellHistoryAbsolutePath(path)}.`);
	}
	return parseShellHistoryEntries(text);
}

async function readShellHistoryEntriesOptional(fsys: any, path: string): Promise<ShellHistoryEntry[]> {
	try {
		return await readShellHistoryEntries(fsys, path);
	} catch {
		return [];
	}
}

function parseShellHistoryEntries(text: string): ShellHistoryEntry[] {
	const entries: ShellHistoryEntry[] = [];
	for (const rawLine of text.split(/\r?\n/)) {
		const line = rawLine.trim();
		if (!line) {
			continue;
		}
		try {
			const value = JSON.parse(line);
			if (isShellHistoryEntry(value)) {
				entries.push(value);
			}
		} catch {
			// Keep the browser picker useful even if one append record is malformed.
		}
	}
	return entries;
}

async function shellHistoryArchivePicks(fsys: any): Promise<ShellHistoryArchivePick[]> {
	const names = (await shellHistoryReadDirNames(fsys, SHELL_HISTORY_ARCHIVE_DIR)).reverse();
	const picks: ShellHistoryArchivePick[] = [];
	for (const name of names) {
		const archiveDir = `${SHELL_HISTORY_ARCHIVE_DIR}/${name}`;
		const commandsPath = `${archiveDir}/commands.jsonl`;
		const manifest = await shellHistoryReadJson(fsys, `${archiveDir}/manifest.json`);
		const count = typeof manifest?.commandCount === "number"
			? manifest.commandCount
			: (await readShellHistoryEntriesOptional(fsys, commandsPath)).length;
		if (count === 0) {
			continue;
		}
		const generatedAt = typeof manifest?.generatedAt === "string" ? manifest.generatedAt : name;
		const firstObserved = typeof manifest?.firstObservedAt === "string" ? manifest.firstObservedAt : undefined;
		const lastObserved = typeof manifest?.lastObservedAt === "string" ? manifest.lastObservedAt : undefined;
		picks.push({
			label: name,
			description: `${count} commands`,
			detail: [generatedAt, firstObserved && lastObserved ? `${firstObserved} to ${lastObserved}` : undefined, shellHistoryAbsolutePath(archiveDir)].filter(Boolean).join(" - "),
			archiveDir,
			commandsPath,
		});
	}
	return picks;
}

async function pickShellHistoryArchive(
	fsys: any,
	target: ShellHistoryArchiveTarget | undefined,
	placeHolder: string,
): Promise<ShellHistoryArchivePick | undefined> {
	const targetedArchive = await shellHistoryArchiveInfoForTarget(fsys, target);
	if (targetedArchive) {
		return shellHistoryArchivePickFromInfo(targetedArchive);
	}
	const picks = await shellHistoryArchivePicks(fsys);
	if (picks.length === 0) {
		throw new Error("No qjs shell history archives yet. Use Archive Shell Command History first.");
	}
	const pick = await vscode.window.showQuickPick(picks, {
		placeHolder,
		matchOnDescription: true,
		matchOnDetail: true,
	});
	return pick || undefined;
}

async function pickShellHistoryArchiveBundle(fsys: any): Promise<ShellHistoryArchiveInfo | undefined> {
	const archives = await shellHistoryArchiveInfos(fsys);
	if (archives.length === 0) {
		throw new Error("No qjs shell history archives yet. Use Archive Shell Command History first.");
	}
	const pick = await vscode.window.showQuickPick(shellHistoryArchiveBundlePicks(archives), {
		placeHolder: "Export qjs shell history archive bundle",
		matchOnDescription: true,
		matchOnDetail: true,
	});
	return pick?.archive;
}

async function shellHistoryArchiveBundleImportPick(
	fsys: any,
	target?: ShellHistoryArchiveTarget,
): Promise<ShellHistoryArchiveBundleImportPick | undefined> {
	const targetedArchive = await shellHistoryArchiveInfoForTarget(fsys, target);
	if (targetedArchive) {
		let bundleText = "";
		try {
			bundleText = await fsys.readText(targetedArchive.bundleJsonPath);
		} catch {
			throw new Error(`Archive ${shellHistoryAbsolutePath(targetedArchive.archiveDir)} has no bundle.json yet. Use Export Bundle first.`);
		}
		return {
			label: targetedArchive.name,
			description: "import archive bundle",
			detail: shellHistoryAbsolutePath(targetedArchive.bundleJsonPath),
			bundleText,
			sourcePath: targetedArchive.bundleJsonPath,
		};
	}
	const picks = await shellHistoryArchiveBundleImportPicks(fsys);
	if (picks.length === 0) {
		throw new Error("No shell archive bundle found. Open a bundle.json editor or use Export Shell Archive Bundle first.");
	}
	const pick = await vscode.window.showQuickPick(picks, {
		placeHolder: "Import qjs shell history archive bundle",
		matchOnDescription: true,
		matchOnDetail: true,
	});
	return pick || undefined;
}

async function shellHistoryArchiveInfoForTarget(
	fsys: any,
	target?: ShellHistoryArchiveTarget,
	archives?: ShellHistoryArchiveInfo[],
): Promise<ShellHistoryArchiveInfo | undefined> {
	const archiveDir = shellHistoryArchiveTargetDir(target);
	if (!archiveDir) {
		return undefined;
	}
	const candidates = archives || await shellHistoryArchiveInfos(fsys);
	const found = candidates.find((archive) => shellHistoryRelativePath(archive.archiveDir) === archiveDir);
	if (!found) {
		throw new Error(`Shell history archive not found: ${shellHistoryAbsolutePath(archiveDir)}`);
	}
	return found;
}

function shellHistoryArchiveTargetDir(target?: ShellHistoryArchiveTarget): string | undefined {
	if (!target || typeof target.archiveDir !== "string" || target.archiveDir.length === 0) {
		return undefined;
	}
	const archiveDir = shellHistoryRelativePath(target.archiveDir);
	if (!archiveDir.startsWith(`${SHELL_HISTORY_ARCHIVE_DIR}/`) || archiveDir === SHELL_HISTORY_ARCHIVE_DIR) {
		throw new Error(`Invalid shell archive target: ${target.archiveDir}`);
	}
	return archiveDir;
}

function shellHistoryArchivePickFromInfo(archive: ShellHistoryArchiveInfo): ShellHistoryArchivePick {
	return {
		label: archive.name,
		description: `${archive.commandCount} commands`,
		detail: [archive.generatedAt, shellHistoryAbsolutePath(archive.archiveDir)].join(" - "),
		archiveDir: archive.archiveDir,
		commandsPath: archive.commandsPath,
	};
}

async function shellHistoryReadDirNames(fsys: any, path: string): Promise<string[]> {
	let entries: unknown;
	try {
		entries = typeof fsys.readDirEntries === "function"
			? await fsys.readDirEntries(path)
			: await fsys.readDir(path);
	} catch {
		return [];
	}
	return serviceEntryNames(entries).filter((name) => name.length > 0);
}

async function shellHistoryReadJson(fsys: any, path: string): Promise<any | undefined> {
	try {
		return JSON.parse(await fsys.readText(path));
	} catch {
		return undefined;
	}
}

async function shellHistoryArchiveInventory(
	fsys: any,
	generatedAt: Date,
	archives?: ShellHistoryArchiveInfo[],
): Promise<ShellHistoryArchiveInventory> {
	const lastRestoredArchiveDir = await shellHistoryLastRestoredArchiveDir(fsys);
	const archiveInfos = archives || await shellHistoryArchiveInfos(fsys, lastRestoredArchiveDir);
	return {
		generatedAt,
		archives: archiveInfos.map((archive) => ({
			...archive,
			wasLastRestored: lastRestoredArchiveDir
				? shellHistoryRelativePath(lastRestoredArchiveDir) === shellHistoryRelativePath(archive.archiveDir)
				: false,
		})),
		lastRestoredArchiveDir,
	};
}

async function shellHistoryArchiveInfos(fsys: any, lastRestoredArchiveDir?: string): Promise<ShellHistoryArchiveInfo[]> {
	const names = await shellHistoryReadDirNames(fsys, SHELL_HISTORY_ARCHIVE_DIR);
	const archives: ShellHistoryArchiveInfo[] = [];
	for (const name of names) {
		const archiveDir = `${SHELL_HISTORY_ARCHIVE_DIR}/${name.replace(/\/$/, "")}`;
		const commandsPath = `${archiveDir}/commands.jsonl`;
		const manifestPath = `${archiveDir}/manifest.json`;
		const manifest = await shellHistoryReadJson(fsys, manifestPath);
		const commandCount = typeof manifest?.commandCount === "number"
			? manifest.commandCount
			: (await readShellHistoryEntriesOptional(fsys, commandsPath)).length;
		if (commandCount === 0) {
			continue;
		}
		const compareJsonPath = `${archiveDir}/${SHELL_HISTORY_COMPARE_JSON_NAME}`;
		const compare = await shellHistoryReadJson(fsys, compareJsonPath);
		const bundleJsonPath = `${archiveDir}/${SHELL_HISTORY_ARCHIVE_BUNDLE_JSON_NAME}`;
		const bundle = await shellHistoryReadJson(fsys, bundleJsonPath);
		const importJsonPath = `${archiveDir}/${SHELL_HISTORY_ARCHIVE_IMPORT_JSON_NAME}`;
		const imported = await shellHistoryReadJson(fsys, importJsonPath);
		const generatedAt = typeof manifest?.generatedAt === "string" ? manifest.generatedAt : name;
		const generatedAtUnixMillis = typeof manifest?.generatedAtUnixMillis === "number"
			? manifest.generatedAtUnixMillis
			: shellHistoryArchiveIdUnixMillis(name) ?? Date.parse(generatedAt);
		archives.push({
			name,
			archiveDir,
			commandsPath,
			indexPath: `${archiveDir}/index.md`,
			manifestPath,
			summaryPath: `${archiveDir}/summary.md`,
			latestMarkdownPath: `${archiveDir}/latest.md`,
			generatedAt,
			generatedAtUnixMillis: Number.isFinite(generatedAtUnixMillis) ? generatedAtUnixMillis : undefined,
			commandCount,
			firstObservedAt: typeof manifest?.firstObservedAt === "string" ? manifest.firstObservedAt : undefined,
			lastObservedAt: typeof manifest?.lastObservedAt === "string" ? manifest.lastObservedAt : undefined,
			compareMarkdownPath: `${archiveDir}/${SHELL_HISTORY_COMPARE_MD_NAME}`,
			compareJsonPath,
			compareGeneratedAt: typeof compare?.generatedAt === "string" ? compare.generatedAt : undefined,
			archivedOnlyCount: typeof compare?.archivedOnlyCount === "number" ? compare.archivedOnlyCount : undefined,
			liveOnlyCount: typeof compare?.liveOnlyCount === "number" ? compare.liveOnlyCount : undefined,
			wasLastRestored: lastRestoredArchiveDir
				? shellHistoryRelativePath(lastRestoredArchiveDir) === shellHistoryRelativePath(archiveDir)
				: false,
			bundleMarkdownPath: `${archiveDir}/${SHELL_HISTORY_ARCHIVE_BUNDLE_MD_NAME}`,
			bundleJsonPath,
			bundleGeneratedAt: typeof bundle?.generatedAt === "string" ? bundle.generatedAt : undefined,
			bundleFileCount: typeof bundle?.fileCount === "number" ? bundle.fileCount : undefined,
			importMarkdownPath: `${archiveDir}/${SHELL_HISTORY_ARCHIVE_IMPORT_MD_NAME}`,
			importJsonPath,
			importGeneratedAt: typeof imported?.generatedAt === "string" ? imported.generatedAt : undefined,
		});
	}
	return archives.sort((left, right) => shellHistoryArchiveSortMillis(right) - shellHistoryArchiveSortMillis(left) || right.name.localeCompare(left.name));
}

async function shellHistoryPersistedArchiveInfos(fsys: any): Promise<ShellHistoryArchiveInfo[]> {
	const value = await shellHistoryReadJson(fsys, SHELL_HISTORY_ARCHIVE_INVENTORY_JSON_PATH);
	if (!value || value.schema !== "wanix.qjs-shell.archive-inventory.v1" || !Array.isArray(value.archives)) {
		return [];
	}
	const archives: ShellHistoryArchiveInfo[] = [];
	for (const archive of value.archives) {
		const parsed = shellHistoryArchiveInfoFromInventoryJson(archive);
		if (parsed) {
			archives.push(parsed);
		}
	}
	return archives.sort((left, right) => shellHistoryArchiveSortMillis(right) - shellHistoryArchiveSortMillis(left) || right.name.localeCompare(left.name));
}

function shellHistoryArchiveInfoFromInventoryJson(source: any): ShellHistoryArchiveInfo | undefined {
	if (!source || typeof source !== "object") {
		return undefined;
	}
	const rawArchiveDir = typeof source.archiveDir === "string"
		? source.archiveDir
		: typeof source.indexPath === "string"
			? source.indexPath.replace(/\/index\.md$/, "")
			: "";
	const archiveDir = shellHistoryRelativePath(rawArchiveDir);
	if (!archiveDir.startsWith(`${SHELL_HISTORY_ARCHIVE_DIR}/`) || archiveDir === SHELL_HISTORY_ARCHIVE_DIR) {
		return undefined;
	}
	const commandCount = typeof source.commandCount === "number" ? source.commandCount : 0;
	if (commandCount <= 0) {
		return undefined;
	}
	const name = typeof source.name === "string" && source.name
		? source.name
		: archiveDir.split("/").pop() || "archive";
	const generatedAt = typeof source.generatedAt === "string" ? source.generatedAt : name;
	const generatedAtUnixMillis = typeof source.generatedAtUnixMillis === "number"
		? source.generatedAtUnixMillis
		: shellHistoryArchiveIdUnixMillis(name) ?? Date.parse(generatedAt);
	return {
		name,
		archiveDir,
		commandsPath: shellHistoryArchiveInventoryPath(source, "commandsPath", `${archiveDir}/commands.jsonl`, archiveDir),
		indexPath: shellHistoryArchiveInventoryPath(source, "indexPath", `${archiveDir}/index.md`, archiveDir),
		manifestPath: shellHistoryArchiveInventoryPath(source, "manifestPath", `${archiveDir}/manifest.json`, archiveDir),
		summaryPath: shellHistoryArchiveInventoryPath(source, "summaryPath", `${archiveDir}/summary.md`, archiveDir),
		latestMarkdownPath: shellHistoryArchiveInventoryPath(source, "latestMarkdownPath", `${archiveDir}/latest.md`, archiveDir),
		generatedAt,
		generatedAtUnixMillis: Number.isFinite(generatedAtUnixMillis) ? generatedAtUnixMillis : undefined,
		commandCount,
		firstObservedAt: typeof source.firstObservedAt === "string" ? source.firstObservedAt : undefined,
		lastObservedAt: typeof source.lastObservedAt === "string" ? source.lastObservedAt : undefined,
		compareMarkdownPath: shellHistoryArchiveInventoryPath(source, "compareMarkdownPath", `${archiveDir}/${SHELL_HISTORY_COMPARE_MD_NAME}`, archiveDir),
		compareJsonPath: shellHistoryArchiveInventoryPath(source, "compareJsonPath", `${archiveDir}/${SHELL_HISTORY_COMPARE_JSON_NAME}`, archiveDir),
		compareGeneratedAt: typeof source.compareGeneratedAt === "string" ? source.compareGeneratedAt : undefined,
		archivedOnlyCount: typeof source.archivedOnlyCount === "number" ? source.archivedOnlyCount : undefined,
		liveOnlyCount: typeof source.liveOnlyCount === "number" ? source.liveOnlyCount : undefined,
		wasLastRestored: source.wasLastRestored === true,
		bundleMarkdownPath: shellHistoryArchiveInventoryPath(source, "bundleMarkdownPath", `${archiveDir}/${SHELL_HISTORY_ARCHIVE_BUNDLE_MD_NAME}`, archiveDir),
		bundleJsonPath: shellHistoryArchiveInventoryPath(source, "bundleJsonPath", `${archiveDir}/${SHELL_HISTORY_ARCHIVE_BUNDLE_JSON_NAME}`, archiveDir),
		bundleGeneratedAt: typeof source.bundleGeneratedAt === "string" ? source.bundleGeneratedAt : undefined,
		bundleFileCount: typeof source.bundleFileCount === "number" ? source.bundleFileCount : undefined,
		importMarkdownPath: shellHistoryArchiveInventoryPath(source, "importMarkdownPath", `${archiveDir}/${SHELL_HISTORY_ARCHIVE_IMPORT_MD_NAME}`, archiveDir),
		importJsonPath: shellHistoryArchiveInventoryPath(source, "importJsonPath", `${archiveDir}/${SHELL_HISTORY_ARCHIVE_IMPORT_JSON_NAME}`, archiveDir),
		importGeneratedAt: typeof source.importGeneratedAt === "string" ? source.importGeneratedAt : undefined,
	};
}

function shellHistoryArchiveInventoryPath(source: any, key: string, fallback: string, archiveDir: string): string {
	const path = shellHistoryRelativePath(typeof source?.[key] === "string" ? source[key] : fallback);
	return path.startsWith(`${archiveDir}/`) ? path : fallback;
}

async function shellHistoryLastRestoredArchiveDir(fsys: any): Promise<string | undefined> {
	const restore = await shellHistoryReadJson(fsys, SHELL_HISTORY_RESTORE_JSON_PATH);
	return typeof restore?.archiveDir === "string" ? shellHistoryRelativePath(restore.archiveDir) : undefined;
}

async function writeShellHistoryArchiveInventory(
	fsys: any,
	inventory: ShellHistoryArchiveInventory,
): Promise<string[]> {
	await fsys.makeDirAll(SHELL_HISTORY_ARCHIVE_DIR);
	await fsys.writeFile(SHELL_HISTORY_ARCHIVE_INVENTORY_JSON_PATH, shellHistoryArchiveInventoryJson(inventory));
	await fsys.writeFile(SHELL_HISTORY_ARCHIVE_INVENTORY_MD_PATH, shellHistoryArchiveInventoryMarkdown(inventory));
	return [SHELL_HISTORY_ARCHIVE_INVENTORY_MD_PATH, SHELL_HISTORY_ARCHIVE_INVENTORY_JSON_PATH, SHELL_HISTORY_ARCHIVE_DIR];
}

function shellHistoryArchiveInventoryJson(inventory: ShellHistoryArchiveInventory): string {
	return `${JSON.stringify({
		schema: "wanix.qjs-shell.archive-inventory.v1",
		generatedAt: inventory.generatedAt.toISOString(),
		archiveRoot: shellHistoryAbsolutePath(SHELL_HISTORY_ARCHIVE_DIR),
		inventoryMarkdownPath: shellHistoryAbsolutePath(SHELL_HISTORY_ARCHIVE_INVENTORY_MD_PATH),
		inventoryJsonPath: shellHistoryAbsolutePath(SHELL_HISTORY_ARCHIVE_INVENTORY_JSON_PATH),
		archiveCount: inventory.archives.length,
		totalCommandCount: inventory.archives.reduce((sum, archive) => sum + archive.commandCount, 0),
		lastRestoredArchiveDir: inventory.lastRestoredArchiveDir ? shellHistoryAbsolutePath(inventory.lastRestoredArchiveDir) : undefined,
		archives: inventory.archives.map(shellHistoryArchiveInfoJson),
	}, null, 2)}\n`;
}

function shellHistoryArchiveInfoJson(archive: ShellHistoryArchiveInfo): Record<string, unknown> {
	return {
		name: archive.name,
		archiveDir: shellHistoryAbsolutePath(archive.archiveDir),
		generatedAt: archive.generatedAt,
		generatedAtUnixMillis: archive.generatedAtUnixMillis,
		commandCount: archive.commandCount,
		firstObservedAt: archive.firstObservedAt,
		lastObservedAt: archive.lastObservedAt,
		indexPath: shellHistoryAbsolutePath(archive.indexPath),
		manifestPath: shellHistoryAbsolutePath(archive.manifestPath),
		commandsPath: shellHistoryAbsolutePath(archive.commandsPath),
		summaryPath: shellHistoryAbsolutePath(archive.summaryPath),
		latestMarkdownPath: shellHistoryAbsolutePath(archive.latestMarkdownPath),
		compared: archive.compareGeneratedAt !== undefined,
		compareGeneratedAt: archive.compareGeneratedAt,
		compareMarkdownPath: shellHistoryAbsolutePath(archive.compareMarkdownPath),
		compareJsonPath: shellHistoryAbsolutePath(archive.compareJsonPath),
		archivedOnlyCount: archive.archivedOnlyCount,
		liveOnlyCount: archive.liveOnlyCount,
		wasLastRestored: archive.wasLastRestored,
		bundled: archive.bundleGeneratedAt !== undefined,
		bundleGeneratedAt: archive.bundleGeneratedAt,
		bundleMarkdownPath: shellHistoryAbsolutePath(archive.bundleMarkdownPath),
		bundleJsonPath: shellHistoryAbsolutePath(archive.bundleJsonPath),
		bundleFileCount: archive.bundleFileCount,
		imported: archive.importGeneratedAt !== undefined,
		importGeneratedAt: archive.importGeneratedAt,
		importMarkdownPath: shellHistoryAbsolutePath(archive.importMarkdownPath),
		importJsonPath: shellHistoryAbsolutePath(archive.importJsonPath),
	};
}

function shellHistoryArchiveInventoryMarkdown(inventory: ShellHistoryArchiveInventory): string {
	return [
		"# qjs Shell Archive Inventory",
		"",
		"Schema: wanix.qjs-shell.archive-inventory.v1",
		`Generated: ${inventory.generatedAt.toISOString()}`,
		`Archive root: ${shellHistoryWanixLink(shellHistoryAbsolutePath(SHELL_HISTORY_ARCHIVE_DIR), SHELL_HISTORY_ARCHIVE_DIR)}`,
		`JSON: ${shellHistoryWanixLink(shellHistoryAbsolutePath(SHELL_HISTORY_ARCHIVE_INVENTORY_JSON_PATH), SHELL_HISTORY_ARCHIVE_INVENTORY_JSON_PATH)}`,
		"",
		"## Counts",
		"",
		`- Archives: ${inventory.archives.length}`,
		`- Archived commands: ${inventory.archives.reduce((sum, archive) => sum + archive.commandCount, 0)}`,
		`- Last restored archive: ${inventory.lastRestoredArchiveDir ? shellHistoryWanixLink(shellHistoryAbsolutePath(`${inventory.lastRestoredArchiveDir}/index.md`), `${inventory.lastRestoredArchiveDir}/index.md`) : "none"}`,
		"",
		"## Archives",
		"",
		...shellHistoryArchiveInventoryRows(inventory.archives),
		"",
		"## Retention",
		"",
		"Use `Prune Shell History Archives` to keep the latest count or recent time window. Pruning deletes archived evidence directories after confirmation, then regenerates this inventory.",
		"",
	].join("\n");
}

function shellHistoryArchiveInventoryRows(archives: ShellHistoryArchiveInfo[]): string[] {
	if (archives.length === 0) {
		return ["No shell history archives yet. Use `Archive Shell Command History` first."];
	}
	const rows = [
		"| Archive | Commands | Range | Compare | Bundle | Import | Restore |",
		"| --- | ---: | --- | --- | --- | --- | --- |",
	];
	for (const archive of archives) {
		const archiveLink = shellHistoryWanixLink(archive.name, archive.indexPath);
		const range = [archive.firstObservedAt, archive.lastObservedAt].filter(Boolean).join(" to ") || archive.generatedAt;
		const compare = archive.compareGeneratedAt
			? `${shellHistoryWanixLink("compared", archive.compareMarkdownPath)} (${archive.archivedOnlyCount ?? "?"} archived-only, ${archive.liveOnlyCount ?? "?"} live-only)`
			: "not compared";
		const bundle = archive.bundleGeneratedAt
			? `${shellHistoryWanixLink("bundle", archive.bundleMarkdownPath)} (${archive.bundleFileCount ?? "?"} files)`
			: "not exported";
		const imported = archive.importGeneratedAt
			? shellHistoryWanixLink("imported", archive.importMarkdownPath)
			: "";
		const restore = archive.wasLastRestored ? "last restored" : "";
		rows.push(`| ${archiveLink} | ${archive.commandCount} | ${range} | ${compare} | ${bundle} | ${imported} | ${restore} |`);
	}
	return rows;
}

function shellHistoryArchiveBundlePicks(archives: ShellHistoryArchiveInfo[]): ShellHistoryArchiveBundlePick[] {
	return archives.map((archive) => ({
		label: archive.name,
		description: `${archive.commandCount} commands${archive.bundleGeneratedAt ? ", already exported" : ""}`,
		detail: [
			archive.generatedAt,
			archive.bundleGeneratedAt ? `bundle: ${archive.bundleGeneratedAt}` : "no bundle yet",
			shellHistoryAbsolutePath(archive.archiveDir),
		].join(" - "),
		archive,
	}));
}

async function shellHistoryArchiveBundleImportPicks(fsys: any): Promise<ShellHistoryArchiveBundleImportPick[]> {
	const picks: ShellHistoryArchiveBundleImportPick[] = [];
	const editor = vscode.window.activeTextEditor;
	if (editor) {
		const text = editor.document.getText();
		const candidate = shellHistoryTryParseArchiveBundle(text);
		if (candidate) {
			const sourcePath = shellHistoryEditorSourcePath(editor);
			picks.push({
				label: "Current editor bundle",
				description: `${candidate.archive.name}, ${candidate.files.length} files`,
				detail: sourcePath ? shellHistoryAbsolutePath(sourcePath) : editor.document.fileName,
				bundleText: text,
				sourcePath,
			});
		}
	}
	for (const archive of await shellHistoryArchiveInfos(fsys)) {
		const text = await shellHistoryReadTextOptional(fsys, archive.bundleJsonPath);
		if (!text) {
			continue;
		}
		if (picks.some((pick) => pick.sourcePath && shellHistoryRelativePath(pick.sourcePath) === shellHistoryRelativePath(archive.bundleJsonPath))) {
			continue;
		}
		picks.push({
			label: archive.name,
			description: `${archive.bundleFileCount ?? "?"} bundled files`,
			detail: shellHistoryAbsolutePath(archive.bundleJsonPath),
			bundleText: text,
			sourcePath: archive.bundleJsonPath,
		});
	}
	return picks;
}

function shellHistoryEditorSourcePath(editor: vscode.TextEditor): string | undefined {
	const uri = editor.document.uri;
	if (uri.scheme === "wanix") {
		return shellHistoryRelativePath(uri.path);
	}
	return undefined;
}

function shellHistoryTryParseArchiveBundle(text: string): ShellHistoryArchiveBundle | undefined {
	try {
		return shellHistoryParseArchiveBundle(text);
	} catch {
		return undefined;
	}
}

function shellHistoryParseArchiveBundle(text: string): ShellHistoryArchiveBundle {
	let value: any;
	try {
		value = JSON.parse(text);
	} catch (error) {
		throw new Error(`Shell archive bundle JSON is invalid: ${error instanceof Error ? error.message : String(error)}`);
	}
	if (value?.schema !== "wanix.qjs-shell.archive-bundle.v1") {
		throw new Error("Shell archive bundle JSON must use schema wanix.qjs-shell.archive-bundle.v1.");
	}
	if (!Array.isArray(value.files)) {
		throw new Error("Shell archive bundle JSON must contain a files array.");
	}
	const archive = shellHistoryArchiveInfoFromBundleJson(value);
	const files: ShellHistoryArchiveBundleFile[] = [];
	for (const file of value.files) {
		if (!file || typeof file.path !== "string" || typeof file.content !== "string") {
			throw new Error("Shell archive bundle files must have string path and content fields.");
		}
		const path = shellHistoryAbsolutePath(shellHistoryValidateBundleFilePath(archive.archiveDir, file.path));
		files.push({
			path,
			bytes: typeof file.bytes === "number" ? file.bytes : shellHistoryTextBytes(file.content),
			content: file.content,
		});
	}
	const generatedAt = typeof value.generatedAt === "string" && Number.isFinite(Date.parse(value.generatedAt))
		? new Date(value.generatedAt)
		: new Date();
	return { archive, files, generatedAt };
}

function shellHistoryArchiveInfoFromBundleJson(value: any): ShellHistoryArchiveInfo {
	const source = value.archive || {};
	const rawArchiveDir = typeof source.archiveDir === "string"
		? source.archiveDir
		: typeof source.indexPath === "string"
			? source.indexPath.replace(/\/index\.md$/, "")
			: "";
	const archiveDir = shellHistoryRelativePath(rawArchiveDir);
	if (!archiveDir.startsWith(`${SHELL_HISTORY_ARCHIVE_DIR}/`) || archiveDir === SHELL_HISTORY_ARCHIVE_DIR) {
		throw new Error("Shell archive bundle target must be under /.wanix/qjs-shell/archive/<id>.");
	}
	const name = typeof source.name === "string" && source.name
		? source.name
		: archiveDir.split("/").pop() || "imported";
	const bundleGeneratedAt = typeof value.generatedAt === "string" ? value.generatedAt : undefined;
	return {
		name,
		archiveDir,
		commandsPath: shellHistoryRelativePath(source.commandsPath || `${archiveDir}/commands.jsonl`),
		indexPath: shellHistoryRelativePath(source.indexPath || `${archiveDir}/index.md`),
		manifestPath: shellHistoryRelativePath(source.manifestPath || `${archiveDir}/manifest.json`),
		summaryPath: shellHistoryRelativePath(source.summaryPath || `${archiveDir}/summary.md`),
		latestMarkdownPath: shellHistoryRelativePath(source.latestMarkdownPath || `${archiveDir}/latest.md`),
		generatedAt: typeof source.generatedAt === "string" ? source.generatedAt : bundleGeneratedAt || name,
		generatedAtUnixMillis: typeof source.generatedAtUnixMillis === "number" ? source.generatedAtUnixMillis : undefined,
		commandCount: typeof source.commandCount === "number" ? source.commandCount : 0,
		firstObservedAt: typeof source.firstObservedAt === "string" ? source.firstObservedAt : undefined,
		lastObservedAt: typeof source.lastObservedAt === "string" ? source.lastObservedAt : undefined,
		compareMarkdownPath: shellHistoryRelativePath(source.compareMarkdownPath || `${archiveDir}/${SHELL_HISTORY_COMPARE_MD_NAME}`),
		compareJsonPath: shellHistoryRelativePath(source.compareJsonPath || `${archiveDir}/${SHELL_HISTORY_COMPARE_JSON_NAME}`),
		compareGeneratedAt: typeof source.compareGeneratedAt === "string" ? source.compareGeneratedAt : undefined,
		archivedOnlyCount: typeof source.archivedOnlyCount === "number" ? source.archivedOnlyCount : undefined,
		liveOnlyCount: typeof source.liveOnlyCount === "number" ? source.liveOnlyCount : undefined,
		wasLastRestored: source.wasLastRestored === true,
		bundleMarkdownPath: `${archiveDir}/${SHELL_HISTORY_ARCHIVE_BUNDLE_MD_NAME}`,
		bundleJsonPath: `${archiveDir}/${SHELL_HISTORY_ARCHIVE_BUNDLE_JSON_NAME}`,
		bundleGeneratedAt,
		bundleFileCount: typeof value.fileCount === "number" ? value.fileCount : undefined,
		importMarkdownPath: `${archiveDir}/${SHELL_HISTORY_ARCHIVE_IMPORT_MD_NAME}`,
		importJsonPath: `${archiveDir}/${SHELL_HISTORY_ARCHIVE_IMPORT_JSON_NAME}`,
	};
}

function shellHistoryValidateBundleFilePath(archiveDir: string, rawPath: string): string {
	const path = shellHistoryRelativePath(rawPath);
	const archivePrefix = `${archiveDir}/`;
	if (!path.startsWith(archivePrefix)) {
		throw new Error(`Bundle file path escapes archive directory: ${rawPath}`);
	}
	if (path.endsWith(`/${SHELL_HISTORY_ARCHIVE_BUNDLE_MD_NAME}`)
		|| path.endsWith(`/${SHELL_HISTORY_ARCHIVE_BUNDLE_JSON_NAME}`)
		|| path.endsWith(`/${SHELL_HISTORY_ARCHIVE_IMPORT_MD_NAME}`)
		|| path.endsWith(`/${SHELL_HISTORY_ARCHIVE_IMPORT_JSON_NAME}`)) {
		throw new Error(`Bundle file path cannot target generated bundle/import reports: ${rawPath}`);
	}
	return path;
}

async function rehydrateShellHistoryArchiveBundle(
	fsys: any,
	bundle: ShellHistoryArchiveBundle,
	sourcePath: string | undefined,
	generatedAt: Date,
): Promise<string[]> {
	await fsys.makeDirAll(bundle.archive.archiveDir);
	for (const file of bundle.files) {
		const path = shellHistoryRelativePath(file.path);
		await fsys.makeDirAll(parentPath(path));
		await fsys.writeFile(path, file.content);
	}
	const bundleJson = shellHistoryArchiveBundleJson({
		...bundle,
		archive: {
			...bundle.archive,
			bundleGeneratedAt: bundle.generatedAt.toISOString(),
			bundleFileCount: bundle.files.length,
		},
	});
	await fsys.writeFile(bundle.archive.bundleJsonPath, bundleJson);
	await fsys.writeFile(bundle.archive.bundleMarkdownPath, shellHistoryArchiveBundleMarkdown(bundle));
	await fsys.writeFile(bundle.archive.importJsonPath, shellHistoryArchiveImportJson(bundle, sourcePath, generatedAt));
	await fsys.writeFile(bundle.archive.importMarkdownPath, shellHistoryArchiveImportMarkdown(bundle, sourcePath, generatedAt));
	return [
		bundle.archive.importMarkdownPath,
		bundle.archive.importJsonPath,
		bundle.archive.bundleMarkdownPath,
		bundle.archive.bundleJsonPath,
		...bundle.files.map((file) => shellHistoryRelativePath(file.path)),
	];
}

function shellHistoryArchiveImportJson(
	bundle: ShellHistoryArchiveBundle,
	sourcePath: string | undefined,
	generatedAt: Date,
): string {
	return `${JSON.stringify({
		schema: "wanix.qjs-shell.archive-import.v1",
		generatedAt: generatedAt.toISOString(),
		sourcePath: sourcePath ? shellHistoryAbsolutePath(sourcePath) : undefined,
		archiveDir: shellHistoryAbsolutePath(bundle.archive.archiveDir),
		bundleGeneratedAt: bundle.generatedAt.toISOString(),
		importMarkdownPath: shellHistoryAbsolutePath(bundle.archive.importMarkdownPath),
		importJsonPath: shellHistoryAbsolutePath(bundle.archive.importJsonPath),
		bundleMarkdownPath: shellHistoryAbsolutePath(bundle.archive.bundleMarkdownPath),
		bundleJsonPath: shellHistoryAbsolutePath(bundle.archive.bundleJsonPath),
		rehydratedFileCount: bundle.files.length,
		rehydratedBytes: shellHistoryBundleBytes(bundle.files),
		rehydratedFiles: bundle.files.map((file) => ({
			path: file.path,
			bytes: file.bytes,
		})),
	}, null, 2)}\n`;
}

function shellHistoryArchiveImportMarkdown(
	bundle: ShellHistoryArchiveBundle,
	sourcePath: string | undefined,
	generatedAt: Date,
): string {
	return [
		"# qjs Shell Archive Bundle Import",
		"",
		"Schema: wanix.qjs-shell.archive-import.v1",
		`Generated: ${generatedAt.toISOString()}`,
		`Source: ${sourcePath ? shellHistoryWanixLink(shellHistoryAbsolutePath(sourcePath), sourcePath) : "current editor"}`,
		`Archive: ${shellHistoryWanixLink(bundle.archive.name, bundle.archive.indexPath)}`,
		`JSON: ${shellHistoryWanixLink(shellHistoryAbsolutePath(bundle.archive.importJsonPath), bundle.archive.importJsonPath)}`,
		`Bundle: ${shellHistoryWanixLink(shellHistoryAbsolutePath(bundle.archive.bundleJsonPath), bundle.archive.bundleJsonPath)}`,
		"",
		"## Counts",
		"",
		`- Rehydrated files: ${bundle.files.length}`,
		`- Rehydrated bytes: ${shellHistoryBytesLabel(shellHistoryBundleBytes(bundle.files))}`,
		`- Bundle generated: ${bundle.generatedAt.toISOString()}`,
		"",
		"## Rehydrated Files",
		"",
		...shellHistoryArchiveBundleFileRows(bundle.files),
		"",
		"This import rewrites archive evidence files from a portable bundle. It does not replay or re-execute shell commands.",
		"",
	].join("\n");
}

async function shellHistoryArchiveBundle(
	fsys: any,
	archive: ShellHistoryArchiveInfo,
	generatedAt: Date,
): Promise<ShellHistoryArchiveBundle> {
	const files: ShellHistoryArchiveBundleFile[] = [];
	for (const path of await shellHistoryArchiveBundleFilePaths(fsys, archive)) {
		const content = await shellHistoryReadTextOptional(fsys, path);
		if (content !== undefined) {
			files.push({
				path: shellHistoryAbsolutePath(path),
				bytes: shellHistoryTextBytes(content),
				content,
			});
		}
	}
	return { archive, files, generatedAt };
}

async function shellHistoryArchiveBundleFilePaths(fsys: any, archive: ShellHistoryArchiveInfo): Promise<string[]> {
	const paths = [
		archive.indexPath,
		archive.manifestPath,
		archive.commandsPath,
		archive.latestMarkdownPath,
		`${archive.archiveDir}/latest.json`,
		archive.summaryPath,
		archive.compareMarkdownPath,
		archive.compareJsonPath,
	];
	const commandDir = `${archive.archiveDir}/commands`;
	for (const name of await shellHistoryReadDirNames(fsys, commandDir)) {
		paths.push(`${commandDir}/${name}`);
	}
	return [...new Set(paths)].filter((path) => !path.endsWith(`/${SHELL_HISTORY_ARCHIVE_BUNDLE_MD_NAME}`) && !path.endsWith(`/${SHELL_HISTORY_ARCHIVE_BUNDLE_JSON_NAME}`));
}

async function shellHistoryReadTextOptional(fsys: any, path: string): Promise<string | undefined> {
	try {
		return await fsys.readText(path);
	} catch {
		return undefined;
	}
}

async function writeShellHistoryArchiveBundle(fsys: any, bundle: ShellHistoryArchiveBundle): Promise<string[]> {
	await fsys.writeFile(bundle.archive.bundleJsonPath, shellHistoryArchiveBundleJson(bundle));
	await fsys.writeFile(bundle.archive.bundleMarkdownPath, shellHistoryArchiveBundleMarkdown(bundle));
	return [bundle.archive.bundleMarkdownPath, bundle.archive.bundleJsonPath];
}

function shellHistoryArchiveBundleJson(bundle: ShellHistoryArchiveBundle): string {
	return `${JSON.stringify({
		schema: "wanix.qjs-shell.archive-bundle.v1",
		generatedAt: bundle.generatedAt.toISOString(),
		archive: shellHistoryArchiveInfoJson(bundle.archive),
		bundleMarkdownPath: shellHistoryAbsolutePath(bundle.archive.bundleMarkdownPath),
		bundleJsonPath: shellHistoryAbsolutePath(bundle.archive.bundleJsonPath),
		fileCount: bundle.files.length,
		totalBytes: shellHistoryBundleBytes(bundle.files),
		files: bundle.files.map((file) => ({
			path: file.path,
			bytes: file.bytes,
			encoding: "utf-8",
			content: file.content,
		})),
	}, null, 2)}\n`;
}

function shellHistoryArchiveBundleMarkdown(bundle: ShellHistoryArchiveBundle): string {
	return [
		"# qjs Shell Archive Bundle",
		"",
		"Schema: wanix.qjs-shell.archive-bundle.v1",
		`Generated: ${bundle.generatedAt.toISOString()}`,
		`Archive: ${shellHistoryWanixLink(bundle.archive.name, bundle.archive.indexPath)}`,
		`JSON bundle: ${shellHistoryWanixLink(shellHistoryAbsolutePath(bundle.archive.bundleJsonPath), bundle.archive.bundleJsonPath)}`,
		"",
		"## Counts",
		"",
		`- Commands: ${bundle.archive.commandCount}`,
		`- Bundled files: ${bundle.files.length}`,
		`- Bundled bytes: ${shellHistoryBytesLabel(shellHistoryBundleBytes(bundle.files))}`,
		`- Compared: ${bundle.archive.compareGeneratedAt ? `${bundle.archive.archivedOnlyCount ?? "?"} archived-only, ${bundle.archive.liveOnlyCount ?? "?"} live-only` : "not compared"}`,
		"",
		"## Files",
		"",
		...shellHistoryArchiveBundleFileRows(bundle.files),
		"",
		"This bundle is a single JSON artifact containing the archive's text files and command evidence. It is meant to be copied or saved outside the current browser session when the audit trail needs to travel.",
		"",
	].join("\n");
}

function shellHistoryArchiveBundleFileRows(files: ShellHistoryArchiveBundleFile[]): string[] {
	if (files.length === 0) {
		return ["- none"];
	}
	return files.map((file) => `- ${shellHistoryWanixLink(file.path, file.path)} (${shellHistoryBytesLabel(file.bytes)})`);
}

function shellHistoryBundleBytes(files: ShellHistoryArchiveBundleFile[]): number {
	return files.reduce((sum, file) => sum + file.bytes, 0);
}

function shellHistoryTextBytes(text: string): number {
	return new TextEncoder().encode(text).length;
}

function shellHistoryBytesLabel(bytes: number): string {
	if (bytes < 1024) {
		return `${bytes} B`;
	}
	if (bytes < 1024 * 1024) {
		return `${(bytes / 1024).toFixed(1)} KiB`;
	}
	return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function shellHistoryArchivePrunePicks(archives: ShellHistoryArchiveInfo[], now: number): ShellHistoryArchivePrunePick[] {
	const countPick = (keepCount: number): ShellHistoryArchivePrunePick => {
		const removed = shellHistoryArchivesToPrune(archives, { label: `Keep latest ${keepCount} archives`, retentionKind: "count", keepCount }, now).length;
		return {
			label: `Keep latest ${keepCount} archives`,
			description: shellHistoryArchivePruneDescription(archives.length, removed),
			detail: "Delete older timestamped archive directories; live shell history is not changed.",
			retentionKind: "count",
			keepCount,
		};
	};
	const agePick = (label: string, ageMs: number): ShellHistoryArchivePrunePick => {
		const removed = shellHistoryArchivesToPrune(archives, { label, retentionKind: "age", ageMs }, now).length;
		return {
			label,
			description: shellHistoryArchivePruneDescription(archives.length, removed),
			detail: "Delete archives older than this window; archives without parseable timestamps are kept.",
			retentionKind: "age",
			ageMs,
		};
	};
	return [
		countPick(5),
		countPick(10),
		countPick(25),
		agePick("Keep archives from last 7 days", 7 * 24 * 60 * 60 * 1000),
		agePick("Keep archives from last 30 days", 30 * 24 * 60 * 60 * 1000),
	];
}

function shellHistoryArchivePruneDescription(total: number, removed: number): string {
	return removed > 0 ? `${total} -> ${total - removed} archives, remove ${removed}` : `${total} archives already fit`;
}

function shellHistorySingleArchivePrunePick(archive: ShellHistoryArchiveInfo): ShellHistoryArchivePrunePick {
	return {
		label: `Prune ${archive.name}`,
		description: `${archive.commandCount} commands`,
		detail: "Delete this timestamped archive directory; live shell history is not changed.",
		retentionKind: "selected",
	};
}

function shellHistoryArchivesToPrune(
	archives: ShellHistoryArchiveInfo[],
	pick: ShellHistoryArchivePrunePick,
	now: number,
): ShellHistoryArchiveInfo[] {
	const sorted = [...archives].sort((left, right) => shellHistoryArchiveSortMillis(right) - shellHistoryArchiveSortMillis(left) || right.name.localeCompare(left.name));
	if (pick.retentionKind === "count") {
		const keepCount = Math.max(1, pick.keepCount ?? sorted.length);
		return sorted.slice(keepCount);
	}
	if (pick.retentionKind === "selected") {
		return [];
	}
	const cutoff = now - Math.max(0, pick.ageMs ?? 0);
	return sorted.filter((archive) => {
		const millis = archive.generatedAtUnixMillis;
		return millis !== undefined && millis < cutoff;
	});
}

async function removeShellHistoryArchiveDir(fsys: any, archiveDir: string): Promise<void> {
	if (shellHistoryRelativePath(archiveDir) === shellHistoryRelativePath(SHELL_HISTORY_ARCHIVE_DIR)) {
		throw new Error("Refusing to remove the shell history archive root");
	}
	if (typeof fsys.removeAll === "function") {
		await fsys.removeAll(archiveDir);
		return;
	}
	await removeShellHistoryCommandArtifacts(fsys, `${archiveDir}/commands`);
	for (const path of [
		`${archiveDir}/${SHELL_HISTORY_COMPARE_JSON_NAME}`,
		`${archiveDir}/${SHELL_HISTORY_COMPARE_MD_NAME}`,
		`${archiveDir}/${SHELL_HISTORY_ARCHIVE_BUNDLE_JSON_NAME}`,
		`${archiveDir}/${SHELL_HISTORY_ARCHIVE_BUNDLE_MD_NAME}`,
		`${archiveDir}/${SHELL_HISTORY_ARCHIVE_IMPORT_JSON_NAME}`,
		`${archiveDir}/${SHELL_HISTORY_ARCHIVE_IMPORT_MD_NAME}`,
		`${archiveDir}/latest.json`,
		`${archiveDir}/latest.md`,
		`${archiveDir}/summary.md`,
		`${archiveDir}/commands.jsonl`,
		`${archiveDir}/manifest.json`,
		`${archiveDir}/index.md`,
	]) {
		try {
			await fsys.remove(path);
		} catch {
			// Missing optional archive artifacts are fine during pruning.
		}
	}
	await fsys.remove(archiveDir);
}

async function writeShellHistoryArchivePruneReport(
	fsys: any,
	generatedAt: Date,
	pick: ShellHistoryArchivePrunePick,
	removed: ShellHistoryArchiveInfo[],
	remaining: ShellHistoryArchiveInfo[],
): Promise<string[]> {
	await fsys.makeDirAll(SHELL_HISTORY_ARCHIVE_DIR);
	await fsys.writeFile(SHELL_HISTORY_ARCHIVE_PRUNE_JSON_PATH, shellHistoryArchivePruneJson(generatedAt, pick, removed, remaining));
	await fsys.writeFile(SHELL_HISTORY_ARCHIVE_PRUNE_MD_PATH, shellHistoryArchivePruneMarkdown(generatedAt, pick, removed, remaining));
	return [SHELL_HISTORY_ARCHIVE_PRUNE_MD_PATH, SHELL_HISTORY_ARCHIVE_PRUNE_JSON_PATH];
}

function shellHistoryArchivePruneJson(
	generatedAt: Date,
	pick: ShellHistoryArchivePrunePick,
	removed: ShellHistoryArchiveInfo[],
	remaining: ShellHistoryArchiveInfo[],
): string {
	return `${JSON.stringify({
		schema: "wanix.qjs-shell.archive-prune.v1",
		generatedAt: generatedAt.toISOString(),
		retention: {
			label: pick.label,
			kind: pick.retentionKind,
			keepCount: pick.keepCount,
			ageMs: pick.ageMs,
		},
		removedArchiveCount: removed.length,
		remainingArchiveCount: remaining.length,
		pruneMarkdownPath: shellHistoryAbsolutePath(SHELL_HISTORY_ARCHIVE_PRUNE_MD_PATH),
		pruneJsonPath: shellHistoryAbsolutePath(SHELL_HISTORY_ARCHIVE_PRUNE_JSON_PATH),
		inventoryMarkdownPath: shellHistoryAbsolutePath(SHELL_HISTORY_ARCHIVE_INVENTORY_MD_PATH),
		inventoryJsonPath: shellHistoryAbsolutePath(SHELL_HISTORY_ARCHIVE_INVENTORY_JSON_PATH),
		removedArchives: removed.map(shellHistoryArchiveInfoJson),
		remainingArchives: remaining.map(shellHistoryArchiveInfoJson),
	}, null, 2)}\n`;
}

function shellHistoryArchivePruneMarkdown(
	generatedAt: Date,
	pick: ShellHistoryArchivePrunePick,
	removed: ShellHistoryArchiveInfo[],
	remaining: ShellHistoryArchiveInfo[],
): string {
	return [
		"# qjs Shell Archive Prune",
		"",
		"Schema: wanix.qjs-shell.archive-prune.v1",
		`Generated: ${generatedAt.toISOString()}`,
		`Retention: ${pick.label}`,
		`JSON: ${shellHistoryWanixLink(shellHistoryAbsolutePath(SHELL_HISTORY_ARCHIVE_PRUNE_JSON_PATH), SHELL_HISTORY_ARCHIVE_PRUNE_JSON_PATH)}`,
		`Inventory: ${shellHistoryWanixLink(shellHistoryAbsolutePath(SHELL_HISTORY_ARCHIVE_INVENTORY_MD_PATH), SHELL_HISTORY_ARCHIVE_INVENTORY_MD_PATH)}`,
		"",
		"## Counts",
		"",
		`- Removed archives: ${removed.length}`,
		`- Remaining archives: ${remaining.length}`,
		`- Removed commands: ${removed.reduce((sum, archive) => sum + archive.commandCount, 0)}`,
		"",
		"## Removed Archives",
		"",
		...shellHistoryArchivePrunedRows(removed),
		"",
		"Pruning deletes archive directories only. It does not change the current live qjs-shell history files.",
		"",
	].join("\n");
}

function shellHistoryArchivePrunedRows(archives: ShellHistoryArchiveInfo[]): string[] {
	if (archives.length === 0) {
		return ["- none"];
	}
	return archives.map((archive) => `- ${archive.name}: ${archive.commandCount} commands, ${archive.generatedAt}`);
}

function shellHistoryArchiveSortMillis(archive: ShellHistoryArchiveInfo): number {
	return archive.generatedAtUnixMillis ?? 0;
}

function shellHistoryArchiveIdUnixMillis(name: string): number | undefined {
	const match = /^(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})(\d{2})(\d{0,3})Z$/.exec(name);
	if (!match) {
		return undefined;
	}
	const [, year, month, day, hour, minute, second, rawMillis] = match;
	const millisText = rawMillis.padEnd(3, "0").slice(0, 3) || "0";
	const millis = Date.UTC(
		Number(year),
		Number(month) - 1,
		Number(day),
		Number(hour),
		Number(minute),
		Number(second),
		Number(millisText),
	);
	return Number.isFinite(millis) ? millis : undefined;
}

function shellHistoryRelativePath(path: string): string {
	return path.replace(/^wanix:\//, "").replace(/^\/+/, "");
}

function shellHistoryArchiveComparison(
	archiveDir: string,
	archiveEntries: ShellHistoryEntry[],
	liveEntries: ShellHistoryEntry[],
	generatedAt: Date,
): ShellHistoryArchiveComparison {
	const archiveArtifacts = shellHistoryArtifacts(archiveEntries, `${archiveDir}/commands`);
	const archiveKeys = shellHistoryOccurrenceKeys(archiveEntries);
	const liveKeys = shellHistoryOccurrenceKeys(liveEntries);
	const liveSet = new Set(liveKeys);
	const archiveSet = new Set(archiveKeys);
	return {
		archiveDir,
		archiveEntries,
		liveEntries,
		generatedAt,
		retained: archiveEntries
			.map((entry, index) => ({ entry, index: index + 1, artifact: archiveArtifacts[index], key: archiveKeys[index] }))
			.filter((item) => liveSet.has(item.key)),
		archivedOnly: archiveEntries
			.map((entry, index) => ({ entry, index: index + 1, artifact: archiveArtifacts[index], key: archiveKeys[index] }))
			.filter((item) => !liveSet.has(item.key)),
		liveOnly: liveEntries
			.map((entry, index) => ({ entry, index: index + 1, key: liveKeys[index] }))
			.filter((item) => !archiveSet.has(item.key)),
	};
}

function shellHistoryOccurrenceKeys(entries: ShellHistoryEntry[]): string[] {
	const counts = new Map<string, number>();
	return entries.map((entry) => {
		const base = shellHistoryEntryKey(entry);
		const count = (counts.get(base) || 0) + 1;
		counts.set(base, count);
		return `${base}\u0000${count}`;
	});
}

function shellHistoryEntryKey(entry: ShellHistoryEntry): string {
	return JSON.stringify({
		observedAtUnixMillis: entry.observedAtUnixMillis,
		taskId: entry.taskId,
		terminalId: entry.terminalId,
		cwd: entry.cwd,
		command: entry.command,
		outcome: {
			status: entry.outcome?.status,
			changed: entry.outcome?.changed,
			exitCode: entry.outcome?.exitCode,
			evidence: entry.outcome?.evidence,
			diagnostic: entry.outcome?.diagnostic,
		},
		operation: {
			kind: entry.operation?.kind,
			status: entry.operation?.status,
			source: entry.operation?.source,
			target: entry.operation?.target,
			paths: entry.operation?.paths || [],
		},
	});
}

async function writeShellHistoryArchiveComparison(fsys: any, comparison: ShellHistoryArchiveComparison): Promise<string[]> {
	const markdownPath = `${comparison.archiveDir}/${SHELL_HISTORY_COMPARE_MD_NAME}`;
	const jsonPath = `${comparison.archiveDir}/${SHELL_HISTORY_COMPARE_JSON_NAME}`;
	await fsys.writeFile(jsonPath, shellHistoryArchiveComparisonJson(comparison, markdownPath, jsonPath));
	await fsys.writeFile(markdownPath, shellHistoryArchiveComparisonMarkdown(comparison, markdownPath, jsonPath));
	return [
		markdownPath,
		jsonPath,
		`${comparison.archiveDir}/index.md`,
		`${comparison.archiveDir}/manifest.json`,
		`${comparison.archiveDir}/summary.md`,
		`${comparison.archiveDir}/commands.jsonl`,
		`${comparison.archiveDir}/commands`,
	];
}

function shellHistoryArchiveComparisonJson(comparison: ShellHistoryArchiveComparison, markdownPath: string, jsonPath: string): string {
	return `${JSON.stringify({
		schema: "wanix.qjs-shell.archive-compare.v1",
		generatedAt: comparison.generatedAt.toISOString(),
		archiveDir: shellHistoryAbsolutePath(comparison.archiveDir),
		markdownPath: shellHistoryAbsolutePath(markdownPath),
		jsonPath: shellHistoryAbsolutePath(jsonPath),
		archiveCommandCount: comparison.archiveEntries.length,
		liveCommandCount: comparison.liveEntries.length,
		retainedInLiveCount: comparison.retained.length,
		archivedOnlyCount: comparison.archivedOnly.length,
		liveOnlyCount: comparison.liveOnly.length,
		archivedOnly: comparison.archivedOnly.map(shellHistoryComparedEntryJson),
		liveOnly: comparison.liveOnly.map(shellHistoryComparedEntryJson),
	}, null, 2)}\n`;
}

function shellHistoryComparedEntryJson(item: ShellHistoryComparedEntry): Record<string, unknown> {
	return {
		index: item.index,
		command: shellHistoryCommand(item.entry),
		status: shellHistoryStatus(item.entry),
		observedAt: formatShellHistoryTime(item.entry.observedAtUnixMillis),
		cwd: item.entry.cwd,
		target: shellHistoryTarget(item.entry),
		evidencePath: item.artifact ? shellHistoryAbsolutePath(item.artifact.path) : undefined,
	};
}

function shellHistoryArchiveComparisonMarkdown(comparison: ShellHistoryArchiveComparison, markdownPath: string, jsonPath: string): string {
	return [
		"# qjs Shell History Archive Compare",
		"",
		"Schema: wanix.qjs-shell.archive-compare.v1",
		`Generated: ${comparison.generatedAt.toISOString()}`,
		`Archive: ${shellHistoryWanixLink(shellHistoryAbsolutePath(`${comparison.archiveDir}/index.md`), `${comparison.archiveDir}/index.md`)}`,
		`JSON: ${shellHistoryWanixLink(shellHistoryAbsolutePath(jsonPath), jsonPath)}`,
		`Markdown: ${shellHistoryAbsolutePath(markdownPath)}`,
		"",
		"## Counts",
		"",
		`- Archive commands: ${comparison.archiveEntries.length}`,
		`- Current live commands: ${comparison.liveEntries.length}`,
		`- Still present in live history: ${comparison.retained.length}`,
		`- Archived only: ${comparison.archivedOnly.length}`,
		`- Live only: ${comparison.liveOnly.length}`,
		"",
		"## Archived Only",
		"",
		...shellHistoryComparedEntryLines(comparison.archivedOnly, "archive"),
		"",
		"## Live Only",
		"",
		...shellHistoryComparedEntryLines(comparison.liveOnly, "live"),
		"",
		"## Archived-Only Outcome Counts",
		"",
		...shellHistoryEntryCountLines(comparison.archivedOnly.map((item) => item.entry), shellHistoryStatusKey),
		"",
		"## Live-Only Outcome Counts",
		"",
		...shellHistoryEntryCountLines(comparison.liveOnly.map((item) => item.entry), shellHistoryStatusKey),
		"",
	].join("\n");
}

function shellHistoryComparedEntryLines(items: ShellHistoryComparedEntry[], source: "archive" | "live", limit = 20): string[] {
	if (items.length === 0) {
		return ["- none"];
	}
	const lines = items.slice(0, limit).map((item) => {
		if (source === "archive" && item.artifact) {
			return `- ${shellHistoryArtifactSummary(item.artifact)}`;
		}
		return `- ${shellHistoryEntrySummary(item.entry)}`;
	});
	const remaining = items.length - limit;
	if (remaining > 0) {
		lines.push(`- ${remaining} more entries in ${source === "archive" ? "archived-only" : "live-only"} set; see JSON for compact details.`);
	}
	return lines;
}

function shellHistoryEntrySummary(entry: ShellHistoryEntry): string {
	const time = formatShellHistoryTime(entry.observedAtUnixMillis) || "unknown time";
	const target = shellHistoryTarget(entry);
	const targetText = target ? ` -> ${shellHistoryWanixLink(target, target)}` : "";
	return `${time} - ${shellHistoryStatus(entry)} - ${shellHistoryCommand(entry)}${targetText}`;
}

function shellHistoryEntryCountLines(entries: ShellHistoryEntry[], keyFor: (entry: ShellHistoryEntry) => string): string[] {
	const counts = new Map<string, number>();
	for (const entry of entries) {
		const key = keyFor(entry);
		counts.set(key, (counts.get(key) || 0) + 1);
	}
	const lines = [...counts.entries()]
		.sort(([leftKey, leftCount], [rightKey, rightCount]) => rightCount - leftCount || leftKey.localeCompare(rightKey))
		.map(([key, count]) => `- ${key}: ${count}`);
	return lines.length ? lines : ["- none"];
}

function shellHistoryRestoreJson(
	archiveDir: string,
	archiveEntries: ShellHistoryEntry[],
	previousLiveEntries: ShellHistoryEntry[],
	generatedAt: Date,
): string {
	return `${JSON.stringify({
		schema: "wanix.qjs-shell.archive-restore.v1",
		generatedAt: generatedAt.toISOString(),
		archiveDir: shellHistoryAbsolutePath(archiveDir),
		restoredCommandCount: archiveEntries.length,
		previousLiveCommandCount: previousLiveEntries.length,
		historyPath: shellHistoryAbsolutePath(SHELL_HISTORY_JSONL_PATH),
		summaryPath: shellHistoryAbsolutePath(SHELL_HISTORY_SUMMARY_MD_PATH),
		latestMarkdownPath: shellHistoryAbsolutePath(SHELL_HISTORY_MD_PATH),
		commandEvidenceDir: shellHistoryAbsolutePath(SHELL_HISTORY_COMMANDS_DIR),
		restoreMarkdownPath: shellHistoryAbsolutePath(SHELL_HISTORY_RESTORE_MD_PATH),
		restoreJsonPath: shellHistoryAbsolutePath(SHELL_HISTORY_RESTORE_JSON_PATH),
		source: {
			archiveIndexPath: shellHistoryAbsolutePath(`${archiveDir}/index.md`),
			archiveCommandsPath: shellHistoryAbsolutePath(`${archiveDir}/commands.jsonl`),
			archiveSummaryPath: shellHistoryAbsolutePath(`${archiveDir}/summary.md`),
		},
		outcomeCounts: shellHistoryEntryCountObject(archiveEntries, shellHistoryStatusKey),
	}, null, 2)}\n`;
}

function shellHistoryRestoreMarkdown(
	archiveDir: string,
	archiveEntries: ShellHistoryEntry[],
	previousLiveEntries: ShellHistoryEntry[],
	generatedAt: Date,
): string {
	const artifacts = shellHistoryArtifacts(archiveEntries);
	return [
		"# qjs Shell History Restore",
		"",
		"Schema: wanix.qjs-shell.archive-restore.v1",
		`Generated: ${generatedAt.toISOString()}`,
		`Archive: ${shellHistoryWanixLink(shellHistoryAbsolutePath(`${archiveDir}/index.md`), `${archiveDir}/index.md`)}`,
		`Restored commands: ${archiveEntries.length}`,
		`Previous live commands: ${previousLiveEntries.length}`,
		`JSON: ${shellHistoryWanixLink(shellHistoryAbsolutePath(SHELL_HISTORY_RESTORE_JSON_PATH), SHELL_HISTORY_RESTORE_JSON_PATH)}`,
		"",
		"## Live Artifacts Rewritten",
		"",
		`- Commands JSONL: ${shellHistoryWanixLink(shellHistoryAbsolutePath(SHELL_HISTORY_JSONL_PATH), SHELL_HISTORY_JSONL_PATH)}`,
		`- Latest history: ${shellHistoryWanixLink(shellHistoryAbsolutePath(SHELL_HISTORY_MD_PATH), SHELL_HISTORY_MD_PATH)}`,
		`- Grouped summary: ${shellHistoryWanixLink(shellHistoryAbsolutePath(SHELL_HISTORY_SUMMARY_MD_PATH), SHELL_HISTORY_SUMMARY_MD_PATH)}`,
		`- Command evidence: ${shellHistoryWanixLink(shellHistoryAbsolutePath(SHELL_HISTORY_COMMANDS_DIR), SHELL_HISTORY_COMMANDS_DIR)}`,
		"",
		"## Restored Outcome Counts",
		"",
		...shellHistoryEntryCountLines(archiveEntries, shellHistoryStatusKey),
		"",
		"## Recent Restored Commands",
		"",
		...shellHistoryRecentLines(artifacts, 12),
		"",
		"This restore replays archived command records into the live history artifacts. It does not re-execute shell commands.",
		"",
	].join("\n");
}

function shellHistoryEntryCountObject(entries: ShellHistoryEntry[], keyFor: (entry: ShellHistoryEntry) => string): Record<string, number> {
	const counts: Record<string, number> = {};
	for (const entry of entries) {
		const key = keyFor(entry);
		counts[key] = (counts[key] || 0) + 1;
	}
	return counts;
}

function shellHistoryCompactPicks(entries: ShellHistoryEntry[], now: number): ShellHistoryCompactPick[] {
	const countPick = (keepCount: number): ShellHistoryCompactPick => {
		const retained = Math.min(entries.length, keepCount);
		return {
			label: `Keep latest ${keepCount} commands`,
			description: shellHistoryCompactDescription(entries.length, retained),
			detail: "Rewrite commands.jsonl, latest history, grouped summary, and per-command evidence files.",
			retentionKind: "count",
			keepCount,
		};
	};
	const agePick = (label: string, ageMs: number): ShellHistoryCompactPick => {
		const retained = compactShellHistoryEntries(entries, { label, retentionKind: "age", ageMs }, now).length;
		return {
			label,
			description: shellHistoryCompactDescription(entries.length, retained),
			detail: "Keep unknown timestamps plus entries observed inside this window; regenerate history artifacts.",
			retentionKind: "age",
			ageMs,
		};
	};
	return [
		countPick(25),
		countPick(100),
		countPick(250),
		agePick("Keep last 24 hours", 24 * 60 * 60 * 1000),
		agePick("Keep last 7 days", 7 * 24 * 60 * 60 * 1000),
	];
}

function shellHistoryCompactDescription(total: number, retained: number): string {
	const removed = total - retained;
	return removed > 0 ? `${total} -> ${retained} commands, remove ${removed}` : `${total} commands already fit`;
}

function compactShellHistoryEntries(entries: ShellHistoryEntry[], pick: ShellHistoryCompactPick, now: number): ShellHistoryEntry[] {
	if (pick.retentionKind === "count") {
		const keepCount = Math.max(1, pick.keepCount ?? entries.length);
		return entries.slice(Math.max(0, entries.length - keepCount));
	}
	const cutoff = now - Math.max(0, pick.ageMs ?? 0);
	const retained = entries.filter((entry) => {
		const observed = shellHistoryObservedMillis(entry);
		return observed === undefined || observed >= cutoff;
	});
	return retained.length > 0 ? retained : entries.slice(-1);
}

async function rewriteShellHistoryArtifacts(
	fsys: any,
	entries: ShellHistoryEntry[],
	generatedAt: Date,
	options: ShellHistoryRewriteOptions = {},
): Promise<string[]> {
	await fsys.makeDirAll(parentPath(SHELL_HISTORY_JSONL_PATH));
	await fsys.writeFile(SHELL_HISTORY_JSONL_PATH, shellHistoryJsonl(entries));
	await fsys.writeFile(SHELL_HISTORY_JSON_PATH, shellHistoryLatestJson(entries, generatedAt));
	await fsys.writeFile(SHELL_HISTORY_MD_PATH, shellHistoryLatestMarkdown(entries, generatedAt, {
		note: options.latestNote || `This Markdown file shows the latest retained commands from ${shellHistoryAbsolutePath(SHELL_HISTORY_JSONL_PATH)}.`,
	}));
	const artifacts = shellHistoryArtifacts(entries);
	await writeShellHistoryCommandArtifacts(fsys, artifacts);
	await fsys.writeFile(SHELL_HISTORY_SUMMARY_MD_PATH, shellHistorySummaryMarkdown(artifacts, generatedAt));
	if (options.removeSelected !== false) {
		try {
			await fsys.remove(SHELL_HISTORY_SELECTED_MD_PATH);
		} catch {
			// A stale selected command may point at an entry removed by a rewrite.
		}
	}
	const paths = await existingShellHistoryPaths(fsys);
	return paths.includes(SHELL_HISTORY_SUMMARY_MD_PATH) ? paths : [SHELL_HISTORY_SUMMARY_MD_PATH, ...paths];
}

async function writeShellHistoryArchiveArtifacts(
	fsys: any,
	entries: ShellHistoryEntry[],
	archiveDir: string,
	generatedAt: Date,
): Promise<string[]> {
	const indexPath = `${archiveDir}/index.md`;
	const manifestPath = `${archiveDir}/manifest.json`;
	const commandsPath = `${archiveDir}/commands.jsonl`;
	const latestJsonPath = `${archiveDir}/latest.json`;
	const latestMarkdownPath = `${archiveDir}/latest.md`;
	const summaryPath = `${archiveDir}/summary.md`;
	const commandDir = `${archiveDir}/commands`;
	const artifacts = shellHistoryArtifacts(entries, commandDir);
	const archive = {
		archiveDir,
		artifacts,
		commandsPath,
		generatedAt,
		indexPath,
		latestJsonPath,
		latestMarkdownPath,
		manifestPath,
		summaryPath,
	};
	const artifactLinks = { summaryPath, latestPath: latestMarkdownPath };
	await fsys.makeDirAll(commandDir);
	await fsys.writeFile(commandsPath, shellHistoryJsonl(entries));
	await fsys.writeFile(latestJsonPath, shellHistoryLatestJson(entries, generatedAt, {
		historyPath: commandsPath,
		latestJsonPath,
		latestMarkdownPath,
	}));
	await fsys.writeFile(latestMarkdownPath, shellHistoryLatestMarkdown(entries, generatedAt, {
		historyPath: commandsPath,
		latestJsonPath,
		latestMarkdownPath,
		note: `This file is the latest-command tail for archived shell history ${shellHistoryAbsolutePath(archiveDir)}. The archive keeps ${entries.length} commands and links each retained command to archived evidence.`,
	}));
	await writeShellHistoryCommandArtifacts(fsys, artifacts, {
		commandDir,
		links: artifactLinks,
		removeExisting: false,
	});
	await fsys.writeFile(summaryPath, shellHistorySummaryMarkdown(artifacts, generatedAt, {
		archiveIndexPath: indexPath,
		archiveManifestPath: manifestPath,
		commandDir,
		historyPath: commandsPath,
		latestPath: latestMarkdownPath,
	}));
	await fsys.writeFile(manifestPath, shellHistoryArchiveManifestJson(archive));
	await fsys.writeFile(indexPath, shellHistoryArchiveIndexMarkdown(archive));
	return [
		indexPath,
		manifestPath,
		summaryPath,
		latestMarkdownPath,
		latestJsonPath,
		commandsPath,
		commandDir,
	];
}

function shellHistoryArchiveManifestJson(archive: ShellHistoryArchive): string {
	const entries = archive.artifacts.map((artifact) => artifact.entry);
	return `${JSON.stringify({
		schema: "wanix.qjs-shell.history-archive.v1",
		generatedAt: archive.generatedAt.toISOString(),
		generatedAtUnixMillis: archive.generatedAt.valueOf(),
		archiveDir: shellHistoryAbsolutePath(archive.archiveDir),
		indexPath: shellHistoryAbsolutePath(archive.indexPath),
		manifestPath: shellHistoryAbsolutePath(archive.manifestPath),
		commandsPath: shellHistoryAbsolutePath(archive.commandsPath),
		summaryPath: shellHistoryAbsolutePath(archive.summaryPath),
		latestJsonPath: shellHistoryAbsolutePath(archive.latestJsonPath),
		latestMarkdownPath: shellHistoryAbsolutePath(archive.latestMarkdownPath),
		commandEvidenceDir: shellHistoryAbsolutePath(`${archive.archiveDir}/commands`),
		commandCount: entries.length,
		firstObservedAt: shellHistoryObservedRange(entries, "first"),
		lastObservedAt: shellHistoryObservedRange(entries, "last"),
		source: {
			historyPath: shellHistoryAbsolutePath(SHELL_HISTORY_JSONL_PATH),
			summaryPath: shellHistoryAbsolutePath(SHELL_HISTORY_SUMMARY_MD_PATH),
			latestMarkdownPath: shellHistoryAbsolutePath(SHELL_HISTORY_MD_PATH),
		},
		artifacts: archive.artifacts.map((artifact) => ({
			index: artifact.index,
			path: shellHistoryAbsolutePath(artifact.path),
			command: shellHistoryCommand(artifact.entry),
			status: shellHistoryStatus(artifact.entry),
			observedAt: formatShellHistoryTime(artifact.entry.observedAtUnixMillis),
		})),
	}, null, 2)}\n`;
}

function shellHistoryArchiveIndexMarkdown(archive: ShellHistoryArchive): string {
	const entries = archive.artifacts.map((artifact) => artifact.entry);
	const firstObserved = shellHistoryObservedRange(entries, "first");
	const lastObserved = shellHistoryObservedRange(entries, "last");
	return [
		"# qjs Shell History Archive",
		"",
		"Schema: wanix.qjs-shell.history-archive.v1",
		`Generated: ${archive.generatedAt.toISOString()}`,
		`Commands: ${entries.length}`,
		`Time range: ${firstObserved || "unknown"} to ${lastObserved || "unknown"}`,
		"",
		"## Archive Artifacts",
		"",
		`- Manifest: ${shellHistoryWanixLink(shellHistoryAbsolutePath(archive.manifestPath), archive.manifestPath)}`,
		`- Commands JSONL: ${shellHistoryWanixLink(shellHistoryAbsolutePath(archive.commandsPath), archive.commandsPath)}`,
		`- Grouped summary: ${shellHistoryWanixLink(shellHistoryAbsolutePath(archive.summaryPath), archive.summaryPath)}`,
		`- Latest tail: ${shellHistoryWanixLink(shellHistoryAbsolutePath(archive.latestMarkdownPath), archive.latestMarkdownPath)}`,
		`- Latest JSON: ${shellHistoryWanixLink(shellHistoryAbsolutePath(archive.latestJsonPath), archive.latestJsonPath)}`,
		`- Command evidence: ${shellHistoryWanixLink(shellHistoryAbsolutePath(`${archive.archiveDir}/commands`), `${archive.archiveDir}/commands`)}`,
		"",
		"## Outcome Counts",
		...shellHistoryCountLines(shellHistoryGroupBy(archive.artifacts, (artifact) => shellHistoryStatusKey(artifact.entry))),
		"",
		"## Recent Commands",
		...shellHistoryRecentLines(archive.artifacts, 12),
		"",
		"This archive is independent of the live shell history files. It is intended to be created before clearing or compacting long-lived browser sessions.",
		"",
	].join("\n");
}

function shellHistoryArchiveId(generatedAt: Date): string {
	return generatedAt.toISOString().replace(/[-:.]/g, "").replace("T", "-").replace("Z", "Z");
}

function shellHistoryJsonl(entries: ShellHistoryEntry[]): string {
	return entries.map((entry) => JSON.stringify(entry)).join("\n") + (entries.length > 0 ? "\n" : "");
}

function shellHistoryLatestJson(entries: ShellHistoryEntry[], generatedAt: Date, paths: ShellHistoryLatestPaths = {}): string {
	return `${JSON.stringify({
		schema: "wanix.qjs-shell.command-history.v1",
		generatedAtUnixMillis: generatedAt.valueOf(),
		historyPath: shellHistoryAbsolutePath(paths.historyPath || SHELL_HISTORY_JSONL_PATH),
		latestJsonPath: shellHistoryAbsolutePath(paths.latestJsonPath || SHELL_HISTORY_JSON_PATH),
		latestMarkdownPath: shellHistoryAbsolutePath(paths.latestMarkdownPath || SHELL_HISTORY_MD_PATH),
		entries: shellHistoryLatestEntries(entries),
	}, null, 2)}\n`;
}

function shellHistoryLatestMarkdown(entries: ShellHistoryEntry[], generatedAt: Date, paths: ShellHistoryLatestPaths = {}): string {
	return [
		"# Wanix qjs Shell Command History",
		"",
		"Schema: `wanix.qjs-shell.command-history.v1`",
		`Generated: ${generatedAt.toISOString()}`,
		`JSONL: \`${shellHistoryAbsolutePath(paths.historyPath || SHELL_HISTORY_JSONL_PATH)}\``,
		`Latest JSON: \`${shellHistoryAbsolutePath(paths.latestJsonPath || SHELL_HISTORY_JSON_PATH)}\``,
		"",
		"## Latest Retained Commands",
		"",
		"| Command | Status | Changed | Target | Evidence |",
		"| --- | --- | --- | --- | --- |",
		...shellHistoryLatestEntries(entries).map(shellHistoryLatestMarkdownRow),
		"",
		paths.note || `This Markdown file shows the latest retained commands from ${shellHistoryAbsolutePath(paths.historyPath || SHELL_HISTORY_JSONL_PATH)}.`,
		"",
	].join("\n");
}

function shellHistoryLatestEntries(entries: ShellHistoryEntry[]): ShellHistoryEntry[] {
	return entries.slice(-SHELL_HISTORY_LATEST_MARKDOWN_LIMIT);
}

function shellHistoryLatestMarkdownRow(entry: ShellHistoryEntry): string {
	const changed = typeof entry.outcome?.changed === "boolean" ? String(entry.outcome.changed) : "";
	return `| ${shellHistoryMarkdownCell(shellHistoryCommand(entry))} | ${shellHistoryMarkdownCell(entry.outcome?.status || entry.operation?.status || "")} | ${shellHistoryMarkdownCell(changed)} | ${shellHistoryMarkdownCell(shellHistoryTarget(entry) || "")} | ${shellHistoryMarkdownCell(entry.outcome?.evidence || "")} |`;
}

function shellHistoryMarkdownCell(value: string): string {
	return value.replace(/[\r\n]/g, " ").replace(/\|/g, "\\|");
}

function isShellHistoryEntry(value: unknown): value is ShellHistoryEntry {
	if (!value || typeof value !== "object") {
		return false;
	}
	const candidate = value as ShellHistoryEntry;
	return typeof candidate.command === "string" || typeof candidate.cwd === "string" || !!candidate.outcome || !!candidate.operation;
}

function shellHistoryPicks(entries: ShellHistoryEntry[]): ShellHistoryPick[] {
	return [...entries].reverse().map((entry) => ({
		label: shellHistoryCommand(entry),
		description: shellHistoryDescription(entry),
		detail: shellHistoryDetail(entry),
		entry,
	}));
}

function shellHistoryCommand(entry: ShellHistoryEntry): string {
	const command = entry.command?.trim();
	return command || "(empty command)";
}

function shellHistoryDescription(entry: ShellHistoryEntry): string {
	const parts = [shellHistoryStatus(entry)];
	if (entry.cwd) {
		parts.push(entry.cwd);
	}
	const target = shellHistoryTarget(entry);
	if (target) {
		parts.push(target);
	}
	return parts.filter(Boolean).join(" - ");
}

function shellHistoryStatus(entry: ShellHistoryEntry): string {
	const status = entry.outcome?.status || entry.operation?.status || "observed";
	const changed = typeof entry.outcome?.changed === "boolean"
		? (entry.outcome.changed ? "changed" : "unchanged")
		: undefined;
	const exitCode = typeof entry.outcome?.exitCode === "number" ? `exit ${entry.outcome.exitCode}` : undefined;
	return [status, changed, exitCode].filter(Boolean).join(" / ");
}

function shellHistoryTarget(entry: ShellHistoryEntry): string | undefined {
	return entry.operation?.target || entry.operation?.source || entry.operation?.paths?.[0];
}

function shellHistoryDetail(entry: ShellHistoryEntry): string {
	return entry.outcome?.diagnostic
		|| entry.outcome?.terminalOutput
		|| entry.outcome?.evidence
		|| entry.operation?.kind
		|| formatShellHistoryTime(entry.observedAtUnixMillis)
		|| "";
}

function shellHistorySelectionMarkdown(entry: ShellHistoryEntry): string {
	const lines = [
		"# qjs Shell History Selection",
		"",
		`Schema: wanix.qjs-shell.selection.v1`,
		`Command: ${shellHistoryCommand(entry)}`,
		`Status: ${shellHistoryStatus(entry)}`,
	];
	const observedAt = formatShellHistoryTime(entry.observedAtUnixMillis);
	if (observedAt) {
		lines.push(`Observed: ${observedAt}`);
	}
	if (entry.cwd) {
		lines.push(`Cwd: ${entry.cwd}`);
	}
	if (entry.taskId) {
		lines.push(`Task: ${entry.taskId}`);
	}
	if (entry.terminalId) {
		lines.push(`Terminal: ${entry.terminalId}`);
	}
	lines.push("", "## Outcome");
	lines.push(`Evidence: ${entry.outcome?.evidence || "unknown"}`);
	lines.push(`Diagnostic: ${entry.outcome?.diagnostic || ""}`);
	if (entry.outcome?.terminalOutput) {
		lines.push("", "### Terminal Output", "```text", entry.outcome.terminalOutput.trimEnd(), "```");
	}
	lines.push("", "## Operation");
	lines.push(`Kind: ${entry.operation?.kind || ""}`);
	lines.push(`Status: ${entry.operation?.status || ""}`);
	if (entry.operation?.source) {
		lines.push(`Source: ${entry.operation.source}`);
	}
	if (entry.operation?.target) {
		lines.push(`Target: ${entry.operation.target}`);
	}
	if (entry.operation?.paths?.length) {
		lines.push("Paths:");
		for (const path of entry.operation.paths) {
			lines.push(`- ${path}`);
		}
	}
	lines.push("", "## Raw JSON", "```json", JSON.stringify(entry, null, 2), "```", "");
	return lines.join("\n");
}

async function writeShellHistoryCommandArtifacts(
	fsys: any,
	artifacts: ShellHistoryArtifact[],
	options: { commandDir?: string; links?: ShellHistoryArtifactLinks; removeExisting?: boolean } = {},
): Promise<void> {
	const commandDir = options.commandDir || SHELL_HISTORY_COMMANDS_DIR;
	if (options.removeExisting !== false) {
		await removeShellHistoryCommandArtifacts(fsys, commandDir);
	}
	await fsys.makeDirAll(commandDir);
	for (const artifact of artifacts) {
		await fsys.writeFile(artifact.path, shellHistoryCommandArtifactMarkdown(artifact, options.links));
	}
}

async function removeShellHistoryCommandArtifacts(fsys: any, commandDir = SHELL_HISTORY_COMMANDS_DIR): Promise<void> {
	if (typeof fsys.removeAll === "function") {
		try {
			await fsys.removeAll(commandDir);
			return;
		} catch {
			// The directory is created only after the summary has been generated.
		}
	}
	let entries: any[] = [];
	try {
		entries = typeof fsys.readDirEntries === "function"
			? await fsys.readDirEntries(commandDir)
			: [];
	} catch {
		return;
	}
	for (const entry of entries) {
		const name = typeof entry === "string" ? entry : entry?.Name;
		if (name) {
			await fsys.remove(`${commandDir}/${name.replace(/\/$/, "")}`);
		}
	}
	try {
		await fsys.remove(commandDir);
	} catch {
		// A missing empty directory is fine here too.
	}
}

function shellHistoryArtifacts(entries: ShellHistoryEntry[], commandDir = SHELL_HISTORY_COMMANDS_DIR): ShellHistoryArtifact[] {
	const width = Math.max(4, String(entries.length).length);
	return entries.map((entry, index) => {
		const number = String(index + 1).padStart(width, "0");
		return {
			entry,
			index: index + 1,
			path: `${commandDir}/${number}.md`,
		};
	});
}

function shellHistoryCommandArtifactMarkdown(
	artifact: ShellHistoryArtifact,
	links: ShellHistoryArtifactLinks = {
		summaryPath: SHELL_HISTORY_SUMMARY_MD_PATH,
		latestPath: SHELL_HISTORY_MD_PATH,
	},
): string {
	const entry = artifact.entry;
	const lines = [
		`# qjs Shell Command ${artifact.index}`,
		"",
		"Schema: wanix.qjs-shell.command-evidence.v1",
		`Command: ${shellHistoryCommand(entry)}`,
		`Status: ${shellHistoryStatus(entry)}`,
		`Summary: ${shellHistoryArtifactSummary(artifact)}`,
		`History summary: ${shellHistoryWanixLink(shellHistoryAbsolutePath(links.summaryPath), links.summaryPath)}`,
		`Latest history: ${shellHistoryWanixLink(shellHistoryAbsolutePath(links.latestPath), links.latestPath)}`,
	];
	const observedAt = formatShellHistoryTime(entry.observedAtUnixMillis);
	if (observedAt) {
		lines.push(`Observed: ${observedAt}`);
	}
	if (entry.cwd) {
		lines.push(`Cwd: ${entry.cwd}`);
	}
	if (entry.taskId) {
		lines.push(`Task: ${entry.taskId}`);
	}
	if (entry.terminalId) {
		lines.push(`Terminal: ${entry.terminalId}`);
	}
	lines.push("", "## Outcome");
	lines.push(`Evidence: ${entry.outcome?.evidence || "unknown"}`);
	lines.push(`Diagnostic: ${entry.outcome?.diagnostic || ""}`);
	if (entry.outcome?.terminalOutput) {
		lines.push("", "### Terminal Output", "```text", entry.outcome.terminalOutput.trimEnd(), "```");
	}
	lines.push("", "## Operation");
	lines.push(`Kind: ${entry.operation?.kind || ""}`);
	lines.push(`Status: ${entry.operation?.status || ""}`);
	if (entry.operation?.source) {
		lines.push(`Source: ${shellHistoryWanixLink(entry.operation.source, entry.operation.source)}`);
	}
	if (entry.operation?.target) {
		lines.push(`Target: ${shellHistoryWanixLink(entry.operation.target, entry.operation.target)}`);
	}
	if (entry.operation?.paths?.length) {
		lines.push("Paths:");
		for (const path of entry.operation.paths) {
			lines.push(`- ${shellHistoryWanixLink(path, path)}`);
		}
	}
	lines.push("", "## Raw JSON", "```json", JSON.stringify(entry, null, 2), "```", "");
	return lines.join("\n");
}

function shellHistorySummaryMarkdown(artifacts: ShellHistoryArtifact[], generatedAt: Date, paths: ShellHistorySummaryPaths = {}): string {
	const entries = artifacts.map((artifact) => artifact.entry);
	const firstObserved = shellHistoryObservedRange(entries, "first");
	const lastObserved = shellHistoryObservedRange(entries, "last");
	const lines = [
		"# qjs Shell History Summary",
		"",
		"Schema: wanix.qjs-shell.history-summary.v1",
		`Generated: ${generatedAt.toISOString()}`,
		`Commands: ${entries.length}`,
		`Time range: ${firstObserved || "unknown"} to ${lastObserved || "unknown"}`,
		`Latest history: ${shellHistoryAbsolutePath(paths.latestPath || SHELL_HISTORY_MD_PATH)}`,
		`Append log: ${shellHistoryAbsolutePath(paths.historyPath || SHELL_HISTORY_JSONL_PATH)}`,
		`Command evidence: ${shellHistoryAbsolutePath(paths.commandDir || SHELL_HISTORY_COMMANDS_DIR)}`,
		...(paths.archiveIndexPath ? [`Archive index: ${shellHistoryAbsolutePath(paths.archiveIndexPath)}`] : []),
		...(paths.archiveManifestPath ? [`Archive manifest: ${shellHistoryAbsolutePath(paths.archiveManifestPath)}`] : []),
		"",
		"## Outcome Counts",
		...shellHistoryCountLines(shellHistoryGroupBy(artifacts, (artifact) => shellHistoryStatusKey(artifact.entry))),
		"",
		"## Changed Counts",
		...shellHistoryCountLines(shellHistoryGroupBy(artifacts, (artifact) => shellHistoryChangedKey(artifact.entry))),
		"",
		"## By Time",
		...shellHistoryGroupLines(shellHistoryGroupBy(artifacts, (artifact) => shellHistoryTimeBucket(artifact.entry)), { includeRange: true }),
		"",
		"## By Cwd",
		...shellHistoryGroupLines(shellHistoryGroupBy(artifacts, (artifact) => artifact.entry.cwd || "(unknown cwd)")),
		"",
		"## By Terminal",
		...shellHistoryGroupLines(shellHistoryGroupBy(artifacts, (artifact) => artifact.entry.terminalId || "(unknown terminal)")),
		"",
		"## By Task",
		...shellHistoryGroupLines(shellHistoryGroupBy(artifacts, (artifact) => artifact.entry.taskId || "(unknown task)")),
		"",
		"## Recent Commands",
		...shellHistoryRecentLines(artifacts, 12),
		"",
	];
	return lines.join("\n");
}

function shellHistoryGroupBy(artifacts: ShellHistoryArtifact[], keyFor: (artifact: ShellHistoryArtifact) => string): Map<string, ShellHistoryArtifact[]> {
	const groups = new Map<string, ShellHistoryArtifact[]>();
	for (const artifact of artifacts) {
		const key = keyFor(artifact);
		const group = groups.get(key);
		if (group) {
			group.push(artifact);
		} else {
			groups.set(key, [artifact]);
		}
	}
	return groups;
}

function shellHistoryCountLines(groups: Map<string, ShellHistoryArtifact[]>): string[] {
	return shellHistorySortedGroups(groups).map(([key, group]) => `- ${key}: ${group.length}`);
}

function shellHistoryGroupLines(
	groups: Map<string, ShellHistoryArtifact[]>,
	options: { includeRange?: boolean } = {},
): string[] {
	const lines: string[] = [];
	for (const [key, group] of shellHistorySortedGroups(groups)) {
		const entries = group.map((artifact) => artifact.entry);
		const changed = entries.filter((entry) => entry.outcome?.changed).length;
		const errors = entries.filter(shellHistoryLooksFailed).length;
		const range = options.includeRange ? `, ${shellHistoryObservedRange(entries, "first") || "unknown"} to ${shellHistoryObservedRange(entries, "last") || "unknown"}` : "";
		lines.push(`- ${key}: ${group.length} commands, ${changed} changed, ${errors} failed${range}`);
		for (const artifact of [...group].slice(-3).reverse()) {
			lines.push(`  - ${shellHistoryArtifactSummary(artifact)}`);
		}
	}
	return lines.length ? lines : ["- no entries"];
}

function shellHistoryRecentLines(artifacts: ShellHistoryArtifact[], limit: number): string[] {
	return [...artifacts].slice(-limit).reverse().map((artifact) => `- ${shellHistoryArtifactSummary(artifact)}`);
}

function shellHistoryArtifactSummary(artifact: ShellHistoryArtifact): string {
	const entry = artifact.entry;
	const time = formatShellHistoryTime(entry.observedAtUnixMillis) || "unknown time";
	const target = shellHistoryTarget(entry);
	const targetText = target ? ` -> ${shellHistoryWanixLink(target, target)}` : "";
	return `${time} - ${shellHistoryStatus(entry)} - ${shellHistoryWanixLink(shellHistoryCommand(entry), artifact.path)}${targetText}`;
}

function shellHistorySortedGroups(groups: Map<string, ShellHistoryArtifact[]>): [string, ShellHistoryArtifact[]][] {
	return [...groups.entries()].sort(([leftKey, leftEntries], [rightKey, rightEntries]) => {
		const byCount = rightEntries.length - leftEntries.length;
		return byCount || leftKey.localeCompare(rightKey);
	});
}

function shellHistoryStatusKey(entry: ShellHistoryEntry): string {
	return entry.outcome?.status || entry.operation?.status || "observed";
}

function shellHistoryChangedKey(entry: ShellHistoryEntry): string {
	if (typeof entry.outcome?.changed !== "boolean") {
		return "unknown";
	}
	return entry.outcome.changed ? "changed" : "unchanged";
}

function shellHistoryTimeBucket(entry: ShellHistoryEntry): string {
	const millis = shellHistoryObservedMillis(entry);
	if (millis === undefined) {
		return "(unknown time)";
	}
	const date = new Date(millis);
	return `${date.toISOString().slice(0, 13)}:00Z`;
}

function shellHistoryObservedRange(entries: ShellHistoryEntry[], edge: "first" | "last"): string | undefined {
	const observed = entries
		.map(shellHistoryObservedMillis)
		.filter((value): value is number => value !== undefined)
		.sort((left, right) => left - right);
	const value = edge === "first" ? observed[0] : observed[observed.length - 1];
	return formatShellHistoryTime(value);
}

function shellHistoryObservedMillis(entry: ShellHistoryEntry): number | undefined {
	const value = entry.observedAtUnixMillis;
	return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function shellHistoryLooksFailed(entry: ShellHistoryEntry): boolean {
	const status = entry.outcome?.status || entry.operation?.status;
	if (typeof entry.outcome?.exitCode === "number" && entry.outcome.exitCode !== 0) {
		return true;
	}
	return !!status && status !== "ok" && status !== "observed";
}

function shellHistoryWanixLink(label: string, path: string): string {
	return `[${shellHistoryMarkdownLabel(label)}](${shellHistoryWanixUri(path)})`;
}

function shellHistoryMarkdownLabel(label: string): string {
	return label.replace(/\\/g, "\\\\").replace(/\[/g, "\\[").replace(/]/g, "\\]");
}

function shellHistoryWanixUri(path: string): string {
	const normalized = path.startsWith("/") ? path : `/${path}`;
	return `wanix:${encodeURI(normalized).replace(/\(/g, "%28").replace(/\)/g, "%29")}`;
}

function shellHistoryAbsolutePath(path: string): string {
	return path.startsWith("/") ? path : `/${path}`;
}

function formatShellHistoryTime(value: unknown): string | undefined {
	if (typeof value !== "number" || !Number.isFinite(value)) {
		return undefined;
	}
	const date = new Date(value);
	if (!Number.isFinite(date.valueOf())) {
		return undefined;
	}
	return date.toISOString();
}

async function publishShellCommandHistoryReport(
	fsys: any,
	systemView: WanixSystemView,
): Promise<void> {
	const paths = await existingShellHistoryPaths(fsys);
	if (paths.length === 0) {
		return;
	}
	const openPath = shellHistoryReportOpenPath(paths);
	publishShellCommandHistoryReportFromPaths(systemView, paths, openPath);
}

function shellHistoryReportOpenPath(paths: string[]): string {
	if (paths.includes(SHELL_HISTORY_MD_PATH)) {
		return SHELL_HISTORY_MD_PATH;
	}
	if (paths.includes(SHELL_HISTORY_SUMMARY_MD_PATH)) {
		return SHELL_HISTORY_SUMMARY_MD_PATH;
	}
	return paths[0];
}

function publishShellCommandHistoryReportFromPaths(
	systemView: WanixSystemView,
	paths: string[],
	openPath: string,
): void {
	systemView.reportPublished("qjs Shell Command History", openPath, {
		kind: "shell",
		description: "served shell outcomes",
		icon: "terminal",
		artifacts: paths,
	});
}

function publishShellCommandHistoryArchiveReport(
	systemView: WanixSystemView,
	archiveDir: string,
	paths: string[],
): void {
	systemView.reportPublished("qjs Shell History Archive", `${archiveDir}/index.md`, {
		kind: "shell",
		description: "exported shell audit",
		icon: "archive",
		artifacts: paths,
	});
}

function publishShellHistoryArchiveInventoryReport(
	systemView: WanixSystemView,
	paths: string[],
): void {
	systemView.reportPublished("qjs Shell Archive Inventory", SHELL_HISTORY_ARCHIVE_INVENTORY_MD_PATH, {
		kind: "shell",
		description: "archive retention",
		icon: "list-tree",
		artifacts: paths,
	});
}

function publishShellArchiveInventoryToSystemView(
	systemView: WanixSystemView,
	archives: WanixShellArchiveRecord[],
): void {
	systemView.shellArchiveInventoryPublished(archives);
}

function publishShellHistoryArchiveBundleReport(
	systemView: WanixSystemView,
	archive: ShellHistoryArchiveInfo,
	paths: string[],
): void {
	systemView.reportPublished("qjs Shell Archive Bundle", archive.bundleMarkdownPath, {
		kind: "shell",
		description: "portable audit bundle",
		icon: "package",
		artifacts: paths,
	});
}

function publishShellHistoryArchiveImportReport(
	systemView: WanixSystemView,
	archive: ShellHistoryArchiveInfo,
	paths: string[],
): void {
	systemView.reportPublished("qjs Shell Archive Import", archive.importMarkdownPath, {
		kind: "shell",
		description: "bundle rehydrated",
		icon: "cloud-download",
		artifacts: paths,
	});
}

function publishShellCommandHistoryArchiveCompareReport(
	systemView: WanixSystemView,
	archiveDir: string,
	paths: string[],
): void {
	systemView.reportPublished("qjs Shell Archive Compare", `${archiveDir}/${SHELL_HISTORY_COMPARE_MD_NAME}`, {
		kind: "shell",
		description: "archive vs live",
		icon: "diff",
		artifacts: paths,
	});
}

function publishShellHistoryArchivePruneReport(
	systemView: WanixSystemView,
	paths: string[],
): void {
	systemView.reportPublished("qjs Shell Archive Prune", SHELL_HISTORY_ARCHIVE_PRUNE_MD_PATH, {
		kind: "shell",
		description: "archive retention",
		icon: "clear-all",
		artifacts: paths,
	});
}

function publishShellCommandHistoryRestoreReport(
	systemView: WanixSystemView,
	paths: string[],
): void {
	systemView.reportPublished("qjs Shell Archive Restore", SHELL_HISTORY_RESTORE_MD_PATH, {
		kind: "shell",
		description: "archive replayed",
		icon: "history",
		artifacts: paths,
	});
}

async function existingShellHistoryPaths(fsys: any): Promise<string[]> {
	const paths = [SHELL_HISTORY_SUMMARY_MD_PATH, SHELL_HISTORY_RESTORE_MD_PATH, SHELL_HISTORY_RESTORE_JSON_PATH, SHELL_HISTORY_COMMANDS_DIR, SHELL_HISTORY_ARCHIVE_DIR, SHELL_HISTORY_ARCHIVE_INVENTORY_MD_PATH, SHELL_HISTORY_ARCHIVE_INVENTORY_JSON_PATH, SHELL_HISTORY_ARCHIVE_PRUNE_MD_PATH, SHELL_HISTORY_ARCHIVE_PRUNE_JSON_PATH, SHELL_HISTORY_SELECTED_MD_PATH, SHELL_HISTORY_MD_PATH, SHELL_HISTORY_JSON_PATH, SHELL_HISTORY_JSONL_PATH];
	const existing: string[] = [];
	for (const path of paths) {
		try {
			await fsys.stat(path);
			existing.push(path);
		} catch {
			// The history appears only after a served qjs-shell command has completed.
		}
	}
	return existing;
}

async function publishDataStoreInventory(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	const generatedAt = new Date();
	await fsys.makeDirAll(".wanix");
	await publishHttpAppDataStores(fsys, systemView);
	systemView.reportPublished("Data Store Index", DATA_STORE_INDEX_MD_PATH, {
		kind: "data",
		description: "live data store inventory",
		icon: "database",
		artifacts: [DATA_STORE_INDEX_MD_PATH, DATA_STORE_INDEX_JSON_PATH],
	});
	await fsys.writeFile(DATA_STORE_INDEX_JSON_PATH, systemView.dataStoreInventoryJson({
		generatedAt,
		markdownPath: DATA_STORE_INDEX_MD_PATH,
		jsonPath: DATA_STORE_INDEX_JSON_PATH,
	}));
	await fsys.writeFile(DATA_STORE_INDEX_MD_PATH, systemView.dataStoreInventoryMarkdown({
		generatedAt,
		markdownPath: DATA_STORE_INDEX_MD_PATH,
		jsonPath: DATA_STORE_INDEX_JSON_PATH,
	}));
	await refreshWanixPaths(bridge, [DATA_STORE_INDEX_MD_PATH, DATA_STORE_INDEX_JSON_PATH]);
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

function bridgeMutationActivity(mutation: WanixBridgeMutation): FilesystemActivity & { label: string; openPath: string; paths: string[] } {
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
		systemView.reportPublished("Cockpit Tour", reportPath, {
			kind: "demo",
			description: "complete",
			icon: "run-all",
			artifacts: [reportPath, ...steps.flatMap((step) => step.artifacts)],
		});
		systemView.filesystemActivity("cockpit tour report written", { path: reportPath });
		await refreshWanixPaths(bridge, [reportPath]);
		await openWanixPath(reportPath);
		vscode.window.showInformationMessage("Wanix OS cockpit tour completed");
	} catch (error) {
		const reportPath = await writeCockpitTourReport(fsys, bridge, startedAt, new Date(), "failed", steps, error);
		systemView.tourStepFailed("OS cockpit tour", error);
		systemView.tourReport(reportPath, "failed");
		systemView.reportPublished("Cockpit Tour", reportPath, {
			kind: "demo",
			description: "failed",
			icon: "run-all",
			artifacts: [reportPath, ...steps.flatMap((step) => step.artifacts)],
		});
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
	let reportPaths: { markdownPath: string; jsonPath: string } | undefined;
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
		const paths = reportPaths || agentReportPaths(target);
		reportPaths = paths;
		const reportDir = parentPath(paths.markdownPath);
		if (reportDir) {
			await fsys.makeDirAll(reportDir);
		}
		const report = {
			target,
			status,
			startedAt,
			completedAt: new Date(),
			steps: trace,
			reportPath: paths.markdownPath,
			jsonPath: paths.jsonPath,
			resultPath,
			error: error instanceof Error ? error.message : error ? String(error) : undefined,
		};
		await fsys.writeFile(paths.markdownPath, agentRepairReportMarkdown(report));
		await fsys.writeFile(paths.jsonPath, agentRepairReportJson(report));
		bridge.refresh(paths.markdownPath);
		bridge.refresh(paths.jsonPath);
		systemView.reportPublished("Agent Repair Report", paths.markdownPath, {
			kind: "agent",
			description: status,
			icon: "tools",
			artifacts: agentRepairArtifacts({
				target,
				reportPath: paths.markdownPath,
				jsonPath: paths.jsonPath,
				resultPath,
				steps: trace,
			}),
		});
		return paths.markdownPath;
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
			const paths = reportPaths || agentReportPaths(target);
			agentStep("write repair report", { icon: "notebook", path: paths.markdownPath, description: paths.jsonPath });
			const path = await writeReport("already repaired");
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
		const paths = reportPaths || agentReportPaths(target);
		agentStep("write repair report", { icon: "notebook", path: paths.markdownPath, description: paths.jsonPath });
		const path = await writeReport("repaired", undefined, resultPath);
		await refreshWanixPaths(bridge, [resultPath, path, paths.jsonPath]);
		await openWanixPath(path);
		vscode.window.showInformationMessage(`Wanix agent repair wrote ${resultPath}: ${result.trim()}`);
	} catch (error) {
		agentStep("repair failed", { icon: "error", description: error instanceof Error ? error.message : String(error) });
		try {
			const paths = reportPaths || agentReportPaths(target);
			agentStep("write failure report", { icon: "notebook", path: paths.markdownPath, description: paths.jsonPath });
			const path = await writeReport("failed", error);
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

async function saveOpenDocument(uri: vscode.Uri, kind: string): Promise<void> {
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

function agentReportPaths(target: TaskRunTarget): { markdownPath: string; jsonPath: string } {
	const markdownPath = agentReportPath(target);
	return {
		markdownPath,
		jsonPath: markdownPath.replace(/\.md$/i, ".json"),
	};
}

function agentRepairReportMarkdown(report: {
	target: TaskRunTarget;
	status: string;
	startedAt: Date;
	completedAt: Date;
	steps: AgentTraceStep[];
	reportPath: string;
	jsonPath: string;
	resultPath?: string;
	error?: string;
}): string {
	const artifacts = agentRepairArtifacts(report);
	return [
		"# Wanix Agent Repair Report",
		"",
		`Status: ${report.status}`,
		`Target: ${displayWanixReportPath(report.target.path)}`,
		"Backend: deterministic local repair",
		"Contract: read file, run task, observe transcript, write file, rerun task, verify filesystem output",
		`Started: ${report.startedAt.toISOString()}`,
		`Completed: ${report.completedAt.toISOString()}`,
		`JSON: ${displayWanixReportPath(report.jsonPath)}`,
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

function agentRepairReportJson(report: {
	target: TaskRunTarget;
	status: string;
	startedAt: Date;
	completedAt: Date;
	steps: AgentTraceStep[];
	reportPath: string;
	jsonPath: string;
	resultPath?: string;
	error?: string;
}): string {
	return `${JSON.stringify({
		schema: "wanix.agent-repair.v1",
		status: report.status,
		backend: "deterministic local repair",
		contract: [
			"readFile",
			"runTask",
			"observeTask",
			"writeFile",
			"diffFiles",
			"runTask",
			"observeTask",
			"writeRepairReport",
			"openPath",
		],
		startedAt: report.startedAt.toISOString(),
		completedAt: report.completedAt.toISOString(),
		target: {
			kind: "qjs",
			path: displayWanixReportPath(report.target.path),
			cwd: displayWanixReportPath(report.target.dir === "." ? "/" : report.target.dir),
			name: report.target.name,
		},
		result: {
			status: report.status,
			resultPath: report.resultPath ? displayWanixReportPath(report.resultPath) : undefined,
			error: report.error,
		},
		reportPath: displayWanixReportPath(report.reportPath),
		jsonPath: displayWanixReportPath(report.jsonPath),
		operations: report.steps.map(agentRepairOperationJson),
		artifacts: agentRepairArtifacts(report).map(displayWanixReportPath),
	}, null, 2)}\n`;
}

function agentRepairArtifacts(report: {
	target: TaskRunTarget;
	reportPath: string;
	jsonPath?: string;
	resultPath?: string;
	steps: AgentTraceStep[];
}): string[] {
	return uniqueReportPaths([
		report.target.path,
		report.resultPath || "",
		report.reportPath,
		report.jsonPath || "",
		...report.steps.flatMap((step) => [
			step.path || "",
			step.beforePath || "",
			step.afterPath || "",
			step.outputPath || "",
			step.metadataPath || "",
		]),
	]);
}

function agentRepairOperationJson(step: AgentTraceStep, index: number): object {
	return {
		index: index + 1,
		tool: agentRepairOperationTool(step.label),
		label: step.label,
		description: step.description,
		taskId: step.taskId,
		exitCode: typeof step.exitCode === "number" ? step.exitCode : undefined,
		path: step.path ? displayWanixReportPath(step.path) : undefined,
		beforePath: step.beforePath ? displayWanixReportPath(step.beforePath) : undefined,
		afterPath: step.afterPath ? displayWanixReportPath(step.afterPath) : undefined,
		transcriptPath: step.outputPath ? displayWanixReportPath(step.outputPath) : undefined,
		metadataPath: step.metadataPath ? displayWanixReportPath(step.metadataPath) : undefined,
	};
}

function agentRepairOperationTool(label: string): string {
	if (label.startsWith("read ")) {
		return "readFile";
	}
	if (label.startsWith("run ") || label.startsWith("rerun ")) {
		return "runTask";
	}
	if (label.startsWith("capture ")) {
		return "observeTask";
	}
	if (label.startsWith("observe ")) {
		return "observeTask";
	}
	if (label.startsWith("snapshot ")) {
		return "writeFile";
	}
	if (label.startsWith("edit ")) {
		return "writeFile";
	}
	if (label.startsWith("diff ")) {
		return "diffFiles";
	}
	if (label.startsWith("verify ")) {
		return "readFile";
	}
	if (label.includes("report")) {
		return "writeRepairReport";
	}
	return "agentStep";
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
	let shellTerminalId: string | undefined;
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
		systemView?.terminalClosed(shellTerminalId || "shell");
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
						if (handleQjsShellSessionMessage(message)) {
							return;
						}
						if (handleQjsShellMutationMessage(message)) {
							return;
						}
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
		},
		setDimensions: (dimensions: vscode.TerminalDimensions) => {
			sendResize(dimensions);
		}
	};

	function handleQjsShellSessionMessage(message: unknown): boolean {
		if (!isQjsShellSessionMessage(message)) {
			return false;
		}
		shellInput.cwd = resolveWanixPath(".", message.cwd);
		if (!shellTaskId) {
			shellTaskId = message.taskId;
			systemView?.taskStarted(shellTaskId, "shell", "shell");
		}
		if (!shellTerminalId) {
			shellTerminalId = message.terminalId;
			systemView?.terminalOpened(shellTerminalId, "Shell");
		}
		systemView?.filesystemActivity(`shell session ${message.taskId} ready`, { path: `#task/${message.taskId}` });
		return true;
	}

	function handleQjsShellMutationMessage(message: unknown): boolean {
		if (!isQjsShellMutationMessage(message)) {
			return false;
		}
		shellInput.cwd = resolveWanixPath(".", message.cwd);
		const activity = shellMutationActivity(message);
		if (activity) {
			notifyFilesystemActivity(onFilesystemActivity, activity);
		}
		return true;
	}
}

type QjsShellSessionMessage = {
	type: "session";
	protocol: "wanix-qjs-shell.v1";
	taskId: string;
	terminalId: string;
	cwd: string;
};

function isQjsShellSessionMessage(message: unknown): message is QjsShellSessionMessage {
	if (!message || typeof message !== "object") {
		return false;
	}
	const candidate = message as Record<string, unknown>;
	return candidate.type === "session"
		&& candidate.protocol === "wanix-qjs-shell.v1"
		&& typeof candidate.taskId === "string"
		&& candidate.taskId.length > 0
		&& typeof candidate.terminalId === "string"
		&& candidate.terminalId.length > 0
		&& typeof candidate.cwd === "string"
		&& candidate.cwd.length > 0;
}

type QjsShellMutationMessage = {
	type: "mutation";
	protocol: "wanix-qjs-shell.v1";
	taskId: string;
	terminalId: string;
	cwd: string;
	paths: string[];
	operations?: QjsShellMutationOperation[];
	historyPaths?: string[];
};

type QjsShellMutationOperation = {
	kind: string;
	command?: string;
	status?: string;
	source?: string;
	target?: string;
	evidence?: string;
	diagnostic?: string;
	exitCode?: number;
	terminalOutput?: string;
	outcome?: QjsShellCommandOutcome;
	paths: string[];
};

type QjsShellCommandOutcome = {
	status?: string;
	changed?: boolean;
	evidence?: string;
	diagnostic?: string;
	exitCode?: number;
	terminalOutput?: string;
};

function isQjsShellMutationMessage(message: unknown): message is QjsShellMutationMessage {
	if (!message || typeof message !== "object") {
		return false;
	}
	const candidate = message as Record<string, unknown>;
	return candidate.type === "mutation"
		&& candidate.protocol === "wanix-qjs-shell.v1"
		&& typeof candidate.taskId === "string"
		&& candidate.taskId.length > 0
		&& typeof candidate.terminalId === "string"
		&& candidate.terminalId.length > 0
		&& typeof candidate.cwd === "string"
		&& candidate.cwd.length > 0
		&& Array.isArray(candidate.paths)
		&& candidate.paths.every((path) => typeof path === "string" && path.length > 0)
		&& (candidate.operations === undefined
			|| (Array.isArray(candidate.operations) && candidate.operations.every(isQjsShellMutationOperation)))
		&& (candidate.historyPaths === undefined
			|| (Array.isArray(candidate.historyPaths) && candidate.historyPaths.every((path) => typeof path === "string" && path.length > 0)));
}

function isQjsShellMutationOperation(operation: unknown): operation is QjsShellMutationOperation {
	if (!operation || typeof operation !== "object") {
		return false;
	}
	const candidate = operation as Record<string, unknown>;
	return typeof candidate.kind === "string"
		&& candidate.kind.length > 0
		&& (candidate.command === undefined || typeof candidate.command === "string")
		&& (candidate.status === undefined || typeof candidate.status === "string")
		&& (candidate.source === undefined || typeof candidate.source === "string")
		&& (candidate.target === undefined || typeof candidate.target === "string")
		&& (candidate.evidence === undefined || typeof candidate.evidence === "string")
		&& (candidate.diagnostic === undefined || typeof candidate.diagnostic === "string")
		&& (candidate.exitCode === undefined || typeof candidate.exitCode === "number")
		&& (candidate.terminalOutput === undefined || typeof candidate.terminalOutput === "string")
		&& (candidate.outcome === undefined || isQjsShellCommandOutcome(candidate.outcome))
		&& Array.isArray(candidate.paths)
		&& candidate.paths.every((path) => typeof path === "string" && path.length > 0);
}

function isQjsShellCommandOutcome(outcome: unknown): outcome is QjsShellCommandOutcome {
	if (!outcome || typeof outcome !== "object") {
		return false;
	}
	const candidate = outcome as Record<string, unknown>;
	return (candidate.status === undefined || typeof candidate.status === "string")
		&& (candidate.changed === undefined || typeof candidate.changed === "boolean")
		&& (candidate.evidence === undefined || typeof candidate.evidence === "string")
		&& (candidate.diagnostic === undefined || typeof candidate.diagnostic === "string")
		&& (candidate.exitCode === undefined || typeof candidate.exitCode === "number")
		&& (candidate.terminalOutput === undefined || typeof candidate.terminalOutput === "string");
}

function shellMutationActivity(message: QjsShellMutationMessage): FilesystemActivity | undefined {
	const historyPaths = uniqueShellPaths(message.historyPaths || []);
	const operation = message.operations?.find((operation) => operation.paths.length > 0)
		|| message.operations?.[0];
	if (operation) {
		return withShellHistoryPaths(shellOperationActivity(operation), historyPaths);
	}
	const unique = uniqueShellPaths(message.paths);
	if (unique.length === 0) {
		if (historyPaths.length === 0) {
			return undefined;
		}
		const openPath = historyPaths[historyPaths.length - 1];
		return { label: "shell command history updated", openPath, paths: historyPaths };
	}
	const openPath = unique[unique.length - 1];
	const label = unique.length === 1
		? `shell changed ${displayWanixPath(openPath)}`
		: `shell changed ${unique.length} paths`;
	return withShellHistoryPaths({ label, openPath, paths: unique }, historyPaths);
}

function withShellHistoryPaths(activity: FilesystemActivity | undefined, historyPaths: string[]): FilesystemActivity | undefined {
	if (!activity || historyPaths.length === 0) {
		return activity;
	}
	return {
		...activity,
		paths: uniqueShellPaths([...(activity.paths || []), ...historyPaths]),
	};
}

function shellOperationActivity(operation: QjsShellMutationOperation): FilesystemActivity | undefined {
	const unique = uniqueShellPaths(operation.paths);
	const target = operation.target && !operation.target.startsWith("#") ? operation.target : undefined;
	const source = operation.source && !operation.source.startsWith("#") ? operation.source : undefined;
	const evidence = operation.outcome?.evidence || operation.evidence;
	if (unique.length === 0) {
		if (operation.status === "unchanged" && (target || source)) {
			const openPath = shellOperationNoChangeOpenPath(operation.kind, source, target);
			return {
				label: shellOperationLabel(operation.kind, source, target, unique, operation.status),
				description: shellOperationDescription(operation),
				evidence,
				openPath,
				paths: [openPath],
			};
		}
		return undefined;
	}
	const openPath = shellOperationOpenPath(operation.kind, source, target, unique);
	const label = shellOperationLabel(operation.kind, source, target, unique, operation.status);
	return { label, description: shellOperationDescription(operation), evidence, openPath, paths: unique };
}

function shellOperationDescription(operation: QjsShellMutationOperation): string | undefined {
	const diagnostic = operation.outcome?.diagnostic || operation.diagnostic;
	const status = operation.outcome?.status;
	const exitCode = operation.outcome?.exitCode ?? operation.exitCode;
	const terminalOutput = operation.outcome?.terminalOutput || operation.terminalOutput;
	const command = operation.command;
	const evidence = shellEvidenceDescription(operation.outcome?.evidence || operation.evidence);
	const exit = typeof exitCode === "number" ? `exit ${exitCode}` : undefined;
	const detail = diagnostic || exit || terminalOutput;
	if (diagnostic && status && status !== "ok") {
		return shellDescriptionParts(`${status}: ${diagnostic}`, command, evidence);
	}
	if (diagnostic) {
		return shellDescriptionParts(diagnostic, command, evidence);
	}
	if (status && status !== "ok") {
		const prefix = detail ? `${status}: ${detail}` : status;
		return shellDescriptionParts(prefix, command, evidence);
	}
	return command && operation.status !== "changed" ? command : undefined;
}

function shellEvidenceDescription(evidence: string | undefined): string | undefined {
	return evidence === "qjs-shell-command-record" ? "recorded by shell" : evidence;
}

function shellDescriptionParts(...parts: (string | undefined)[]): string {
	return parts.filter((part): part is string => Boolean(part)).join(" · ");
}

function shellOperationOpenPath(kind: string, source: string | undefined, target: string | undefined, paths: string[]): string {
	if (kind === "rm" || kind === "rmdir") {
		return parentPath(target || source || paths[0]) || "/";
	}
	if (kind === "mv" && target) {
		return target;
	}
	return target || paths[paths.length - 1];
}

function shellOperationNoChangeOpenPath(kind: string, source: string | undefined, target: string | undefined): string {
	const candidate = target || source;
	return candidate ? parentPath(candidate) || "/" : "/";
}

function shellOperationLabel(kind: string, source: string | undefined, target: string | undefined, paths: string[], status?: string): string {
	const suffix = shellOperationStatusSuffix(status);
	if (kind === "mv" && source && target) {
		return `shell mv ${displayWanixPath(source)} -> ${displayWanixPath(target)}${suffix}`;
	}
	if (kind === "cp" && source && target) {
		return `shell cp ${displayWanixPath(source)} -> ${displayWanixPath(target)}${suffix}`;
	}
	if (kind === "redirect" && paths.length > 1) {
		return `shell redirect ${paths.length} paths${suffix}`;
	}
	return `shell ${kind} ${displayWanixPath(target || source || paths[paths.length - 1])}${suffix}`;
}

function shellOperationStatusSuffix(status: string | undefined): string {
	if (!status || status === "changed") {
		return "";
	}
	if (status === "unchanged") {
		return " no change";
	}
	return ` ${status}`;
}

function uniqueShellPaths(paths: string[]): string[] {
	return [...new Set(paths.filter((path) => path && !path.startsWith("#")))];
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
