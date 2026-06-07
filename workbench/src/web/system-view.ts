import * as vscode from 'vscode';

export type WanixSystemConfig = {
	drivers?: string[];
	httpApp?: {
		route?: string;
		source?: string;
		status?: string;
		protocol?: string;
	};
	ns?: {
		task: string;
		term: string;
	};
	p9?: {
		websocket?: string;
	};
	v86?: {
		launchUrl?: string;
		boot?: {
			ready?: boolean;
			missing?: string[];
		};
		rootfs?: {
			status?: string;
			ready?: boolean;
			missing?: string[];
		};
	};
	qjsShellUrl?: string;
}

type TaskStatus = "running" | "exited" | "closed";
type TerminalStatus = "attached" | "closed";

type TaskRecord = {
	id: string;
	kind: string;
	label: string;
	status: TaskStatus;
	exitCode?: number;
	sourcePath?: string;
	outputPath?: string;
	metadataPath?: string;
	serviceObserved?: boolean;
};

type TerminalRecord = {
	id: string;
	label: string;
	status: TerminalStatus;
	serviceObserved?: boolean;
};

export type WanixServiceTask = {
	id: string;
	kind: string;
	label: string;
	exit?: string;
	exitCode?: number;
};

export type WanixServiceTerminal = {
	id: string;
	label: string;
};

type NamespaceRecord = {
	path: string;
	label: string;
};

type RouteRecord = {
	id: string;
	label: string;
	description: string;
	protocol: string;
	command: vscode.Command;
	contextValue?: string;
	previewStatus?: string;
	previewPath?: string;
};

type RouteRunRecord = {
	id: number;
	routeId: string;
	label: string;
	status: string;
	url?: string;
	sourcePath?: string;
	previewPath?: string;
	artifacts?: RouteRunArtifact[];
};

type RouteRunArtifact = {
	label: string;
	path: string;
	icon?: string;
};

type ActivityRecord = {
	id: number;
	label: string;
	path?: string;
	paths?: string[];
};

type AgentRecord = {
	id: number;
	label: string;
	description?: string;
	icon?: string;
	path?: string;
	beforePath?: string;
	afterPath?: string;
};

type CategoryId = "actions" | "drivers" | "tasks" | "terminals" | "namespace" | "routes" | "routeRuns" | "agent" | "activity";

type SystemTreeItem =
	| { type: "category"; id: CategoryId; label: string }
	| { type: "leaf"; id: string; label: string; description?: string; icon?: vscode.ThemeIcon; command?: vscode.Command; contextValue?: string; taskId?: string; sourcePath?: string; outputPath?: string; metadataPath?: string; path?: string; beforePath?: string; afterPath?: string; children?: SystemTreeItem[] };

const CATEGORIES: Array<SystemTreeItem & { type: "category" }> = [
	{ type: "category", id: "actions", label: "Actions" },
	{ type: "category", id: "activity", label: "Activity" },
	{ type: "category", id: "routes", label: "Routes" },
	{ type: "category", id: "routeRuns", label: "Route Runs" },
	{ type: "category", id: "agent", label: "Agent" },
	{ type: "category", id: "tasks", label: "Tasks" },
	{ type: "category", id: "terminals", label: "Terminals" },
	{ type: "category", id: "namespace", label: "Namespace" },
	{ type: "category", id: "drivers", label: "Drivers" },
];

export class WanixSystemView implements vscode.TreeDataProvider<SystemTreeItem>, vscode.Disposable {
	private readonly emitter = new vscode.EventEmitter<SystemTreeItem | undefined>();
	private readonly disposables: vscode.Disposable[] = [this.emitter];
	private drivers: string[] = [];
	private tasks = new Map<string, TaskRecord>();
	private terminals = new Map<string, TerminalRecord>();
	private namespace: NamespaceRecord[] = [];
	private routes: RouteRecord[] = [];
	private routeRuns: RouteRunRecord[] = [];
	private agent: AgentRecord[] = [];
	private activity: ActivityRecord[] = [];
	private serviceTaskIds = new Set<string>();
	private serviceTerminalIds = new Set<string>();
	private hasHttpApp = false;
	private hasV86 = false;
	private nextActivityId = 1;
	private nextRouteRunId = 1;
	private nextAgentId = 1;

	readonly onDidChangeTreeData = this.emitter.event;

	configure(config: WanixSystemConfig): void {
		this.drivers = [...new Set(config.drivers || [])].sort();
		this.namespace = [
			{ path: "/", label: config.p9?.websocket ? "local served root" : "workspace root" },
		];
		if (config.ns?.task) {
			this.namespace.push({ path: config.ns.task, label: "task service" });
		}
		if (config.ns?.term) {
			this.namespace.push({ path: config.ns.term, label: "terminal device" });
		}
		if (config.qjsShellUrl) {
			this.addActivity("qjs shell route discovered");
		}
		this.hasHttpApp = Boolean(config.httpApp?.route && config.httpApp.status !== "disabled");
		this.hasV86 = Boolean(config.v86?.launchUrl);
		this.routes = routeRecords(config);
		if (this.hasHttpApp) {
			this.addActivity("http app route discovered");
		}
		if (this.hasV86) {
			this.addActivity("direct-v86 route discovered");
		}
		this.refresh();
	}

	register(context: vscode.ExtensionContext): void {
		this.disposables.push(vscode.window.registerTreeDataProvider("wanix.system", this));
		context.subscriptions.push(this);
	}

	taskStarted(id: string, kind: string, label: string, options: { sourcePath?: string; outputPath?: string; metadataPath?: string } = {}): void {
		const existing = this.tasks.get(id);
		this.tasks.set(id, {
			id,
			kind,
			label,
			status: "running",
			sourcePath: options.sourcePath || existing?.sourcePath,
			outputPath: options.outputPath || existing?.outputPath,
			metadataPath: options.metadataPath || existing?.metadataPath,
			serviceObserved: existing?.serviceObserved,
		});
		this.addActivity(`${taskDisplayName({ kind, label })} started`);
		this.refresh();
	}

	taskExited(id: string, code?: number): void {
		const task = this.tasks.get(id);
		if (!task) {
			return;
		}
		task.status = "exited";
		task.exitCode = code;
		this.addActivity(`${taskDisplayName(task)} exited ${formatExitCode(code)}`);
		this.refresh();
	}

	taskClosed(id: string): void {
		const task = this.tasks.get(id);
		if (!task) {
			return;
		}
		task.status = "closed";
		this.addActivity(`${taskDisplayName(task)} closed`);
		this.refresh();
	}

	terminalOpened(id: string, label: string): void {
		const existing = this.terminals.get(id);
		this.terminals.set(id, { id, label, status: "attached", serviceObserved: existing?.serviceObserved });
		this.addActivity(`terminal ${label} attached`);
		this.refresh();
	}

	terminalClosed(id: string): void {
		const terminal = this.terminals.get(id);
		if (!terminal) {
			return;
		}
		terminal.status = "closed";
		this.addActivity(`terminal ${terminal.label} closed`);
		this.refresh();
	}

	filesystemActivity(label = "filesystem activity", options: { path?: string; paths?: string[] } = {}): void {
		this.addActivity(label, options);
		this.refresh();
	}

	observeServiceState(snapshot: { tasks: WanixServiceTask[]; terminals: WanixServiceTerminal[] }): void {
		let changed = false;
		changed = this.observeServiceTasks(snapshot.tasks) || changed;
		changed = this.observeServiceTerminals(snapshot.terminals) || changed;
		if (changed) {
			this.refresh();
		}
	}

	agentStarted(label: string): void {
		this.agent = [];
		this.nextAgentId = 1;
		this.agentStep(label, { icon: "tools" });
	}

	agentStep(label: string, options: { description?: string; icon?: string; path?: string; beforePath?: string; afterPath?: string } = {}): void {
		this.agent.push({
			id: this.nextAgentId++,
			label,
			description: options.description,
			icon: options.icon,
			path: options.path,
			beforePath: options.beforePath,
			afterPath: options.afterPath,
		});
		this.addActivity(`agent ${label}`);
		this.refresh();
	}

	clearFinishedRows(): { tasks: number; terminals: number } {
		const tasks = removeMapEntries(this.tasks, (task) => task.status !== "running");
		const terminals = removeMapEntries(this.terminals, (terminal) => terminal.status === "closed");
		this.addActivity(tasks || terminals
			? `cleared ${formatClearCount(tasks, "task")} and ${formatClearCount(terminals, "terminal")}`
			: "no finished rows to clear");
		this.refresh();
		return { tasks, terminals };
	}

	routePreviewed(id: string, preview: { status: number; statusText?: string; previewPath: string; sourcePath?: string; url?: string; label?: string; artifacts?: RouteRunArtifact[] }): void {
		const route = this.routes.find((candidate) => candidate.id === id);
		if (!route) {
			return;
		}
		route.previewStatus = formatHttpStatus(preview.status, preview.statusText);
		route.previewPath = preview.previewPath;
		const label = preview.label || route.label;
		this.routeRuns.unshift({
			id: this.nextRouteRunId++,
			routeId: route.id,
			label,
			status: route.previewStatus,
			url: preview.url,
			sourcePath: preview.sourcePath,
			previewPath: preview.previewPath,
			artifacts: preview.artifacts,
		});
		this.routeRuns = this.routeRuns.slice(0, 8);
		this.addActivity(`${label} route task ${route.previewStatus}`);
		this.refresh();
	}

	getTreeItem(element: SystemTreeItem): vscode.TreeItem {
		if (element.type === "category") {
			const item = new vscode.TreeItem(element.label, vscode.TreeItemCollapsibleState.Expanded);
			item.id = element.id;
			item.iconPath = categoryIcon(element.id);
			return item;
		}
		const item = new vscode.TreeItem(element.label, element.children?.length
			? vscode.TreeItemCollapsibleState.Collapsed
			: vscode.TreeItemCollapsibleState.None);
		item.description = element.description;
		item.tooltip = treeItemTooltip(element);
		item.iconPath = element.icon;
		item.command = element.command;
		item.contextValue = element.contextValue;
		item.id = element.id;
		return item;
	}

	getChildren(element?: SystemTreeItem): vscode.ProviderResult<SystemTreeItem[]> {
		if (!element) {
			return CATEGORIES;
		}
		if (element.type === "leaf") {
			return element.children || [];
		}
		switch (element.id) {
			case "actions":
				return this.actionItems();
			case "drivers":
				return this.drivers.length > 0
					? this.drivers.map((driver) => leaf(`driver:${driver}`, driver, undefined, "symbol-method"))
					: [leaf("drivers:empty", "no drivers advertised")];
			case "tasks":
				return this.taskItems();
			case "terminals":
				return this.terminalItems();
			case "namespace":
				return this.namespace.map((entry) => leaf(`namespace:${entry.path}`, entry.path, entry.label, namespaceIcon(entry.path), {
					command: "workbench.openWanixPath",
					title: "Open Wanix Path",
					arguments: [entry.path],
				}, "wanixNamespacePath", { path: entry.path }));
			case "routes":
				return this.routes.length > 0
					? this.routes.map((route) => leaf(
						`route:${route.id}`,
						route.label,
						routeDescription(route),
						route.id === "direct-v86" ? "vm" : "globe",
						route.command,
						route.contextValue || (route.previewPath ? "wanixHttpRouteWithPreview" : "wanixHttpRoute"),
						{ path: route.previewPath },
					))
					: [leaf("routes:empty", "no routes advertised")];
			case "routeRuns":
				return this.routeRunItems();
			case "agent":
				return this.agentItems();
			case "activity":
				return this.activity.length > 0
					? this.activity.map((entry) => activityItem(entry))
					: [leaf("activity:empty", "no activity yet")];
		}
	}

	dispose(): void {
		for (const disposable of this.disposables.splice(0)) {
			disposable.dispose();
		}
	}

	private taskItems(): SystemTreeItem[] {
		const tasks = [...this.tasks.values()].sort((left, right) => Number(left.id) - Number(right.id));
		if (tasks.length === 0) {
			return [leaf("tasks:empty", "no observed tasks")];
		}
		return tasks.map((task) => {
			const statusDescription = task.status === "exited"
				? `exited ${formatExitCode(task.exitCode)}`
				: task.status;
			const description = task.serviceObserved ? `${statusDescription} · #task` : statusDescription;
			const children = taskArtifactItems(task);
			const contextValue = task.sourcePath || task.outputPath || task.metadataPath ? "wanixTaskWithArtifacts" : "wanixTask";
			return leaf(`task:${task.id}`, `${task.id} ${taskDisplayName(task)}`, description, taskIcon(task.status), undefined, contextValue, {
				taskId: task.id,
				sourcePath: task.sourcePath,
				outputPath: task.outputPath,
				metadataPath: task.metadataPath,
				path: `#task/${task.id}`,
				children,
			});
		});
	}

	private agentItems(): SystemTreeItem[] {
		if (this.agent.length === 0) {
			return [leaf("agent:empty", "no agent steps yet")];
		}
		return this.agent.map((entry) => {
			if (entry.beforePath && entry.afterPath) {
				return leaf(`agent:${entry.id}`, entry.label, entry.description, entry.icon || "diff", {
					command: "workbench.openWanixDiff",
					title: "Open Wanix Diff",
					arguments: [{ beforePath: entry.beforePath, afterPath: entry.afterPath }],
				}, "wanixAgentDiff", { beforePath: entry.beforePath, afterPath: entry.afterPath });
			}
			return leaf(
				`agent:${entry.id}`,
				entry.label,
				entry.description,
				entry.icon || "tools",
				entry.path ? {
					command: "workbench.openWanixPath",
					title: "Open Wanix Path",
					arguments: [entry.path],
				} : undefined,
				entry.path ? "wanixAgentArtifact" : undefined,
				{ path: entry.path },
			);
		});
	}

	private routeRunItems(): SystemTreeItem[] {
		if (this.routeRuns.length === 0) {
			return [leaf("route-runs:empty", "no route runs yet")];
		}
		return this.routeRuns.map((run) => leaf(
			`route-run:${run.id}`,
			`${run.id} ${run.label}`,
			routeRunDescription(run),
			"globe",
			run.previewPath ? {
				command: "workbench.openWanixPath",
				title: "Open Wanix Path",
				arguments: [run.previewPath],
			} : undefined,
			run.previewPath ? "wanixRouteRun" : undefined,
			{
				path: run.previewPath,
				children: routeRunArtifactItems(run),
			},
		));
	}

	private terminalItems(): SystemTreeItem[] {
		const terminals = [...this.terminals.values()].sort((left, right) => Number(left.id) - Number(right.id));
		if (terminals.length === 0) {
			return [leaf("terminals:empty", "no observed terminals")];
		}
		return terminals.map((terminal) => {
			const label = terminal.id === "shell" ? terminal.label : `${terminal.id} ${terminal.label}`;
			const path = terminal.id === "shell" ? undefined : `#term/${terminal.id}`;
			const description = terminal.serviceObserved ? `${terminal.status} · #term` : terminal.status;
			const contextValue = path ? "wanixTerminalService" : undefined;
			return leaf(`terminal:${terminal.id}`, label, description, "terminal", undefined, contextValue, { path });
		});
	}

	private actionItems(): SystemTreeItem[] {
		const items = [
			actionLeaf("action:new-qjs", "New qjs Script", "qjs", "new-file", "workbench.newQjsScript"),
			actionLeaf("action:run-wasm-starter", "Run WASM Starter", "wasm", "play", "workbench.runWasmStarter"),
			actionLeaf("action:install-wasm-starter", "Install WASM Starter", "wasm", "package", "workbench.installWasmStarter"),
			actionLeaf("action:run-duet", "Run JS and WASM Duet Demo", "qjs + wasm", "run-all", "workbench.runDuetDemo"),
			actionLeaf("action:install-agent-repair", "Install Agent Repair Demo", "agent", "bug", "workbench.installAgentRepairDemo"),
		];
		if (this.hasV86) {
			items.splice(4, 0,
				actionLeaf("action:v86-shared", "Open v86 Shared Files Demo", "linux", "vm", "workbench.openV86SharedDemo"),
				actionLeaf("action:direct-v86", "Open direct-v86 VM", "browser", "vm-running", "workbench.openDirectV86"),
			);
		}
		if (this.hasHttpApp) {
			items.splice(4, 0,
				actionLeaf("action:preview-http", "Preview HTTP App Demo", "http", "globe", "workbench.openHttpAppDemo"),
				actionLeaf("action:preview-http-counter", "Preview HTTP Counter Demo", "stateful http", "server-process", "workbench.openHttpCounterDemo"),
				actionLeaf("action:open-http-handler", "Open HTTP App Handler", "http", "go-to-file", "workbench.openHttpAppHandler"),
			);
		}
		items.push(actionLeaf("action:clear-finished", "Clear Finished Rows", "sidebar", "clear-all", "workbench.clearFinishedSystemRows"));
		return items;
	}

	private addActivity(label: string, options: { path?: string; paths?: string[] } = {}): void {
		const paths = uniquePaths(options.paths || (options.path ? [options.path] : []));
		const path = options.path || paths[paths.length - 1];
		if (this.activity[0]?.label === label && samePaths(this.activity[0].paths, paths) && this.activity[0].path === path) {
			return;
		}
		this.activity.unshift({ id: this.nextActivityId++, label, path, paths });
		this.activity = this.activity.slice(0, 12);
	}

	private observeServiceTasks(tasks: WanixServiceTask[]): boolean {
		let changed = false;
		const nextIds = new Set(tasks.map((task) => task.id));
		for (const task of tasks) {
			const status: TaskStatus = task.exit?.trim() ? "exited" : "running";
			const existing = this.tasks.get(task.id);
			if (!existing) {
				this.tasks.set(task.id, {
					id: task.id,
					kind: task.kind,
					label: task.label,
					status,
					exitCode: task.exitCode,
					serviceObserved: true,
				});
				this.addActivity(`service task ${task.id} observed`);
				changed = true;
				continue;
			}
			if (shouldReplaceServiceKind(existing.kind, task.kind)) {
				existing.kind = task.kind;
				changed = true;
			}
			if (shouldReplaceServiceLabel(existing.label, existing.kind, task.label)) {
				existing.label = task.label;
				changed = true;
			}
			if (!existing.serviceObserved) {
				existing.serviceObserved = true;
				this.addActivity(`service task ${task.id} observed`);
				changed = true;
			}
			if (existing.status !== status) {
				existing.status = status;
				this.addActivity(status === "exited"
					? `service task ${task.id} exited ${formatExitCode(task.exitCode)}`
					: `service task ${task.id} running`);
				changed = true;
			}
			if (existing.exitCode !== task.exitCode) {
				existing.exitCode = task.exitCode;
				changed = true;
			}
		}
		for (const id of this.serviceTaskIds) {
			if (nextIds.has(id)) {
				continue;
			}
			const existing = this.tasks.get(id);
			if (!existing) {
				continue;
			}
			if (existing.sourcePath || existing.outputPath || existing.metadataPath) {
				if (existing.status !== "closed") {
					existing.status = "closed";
					this.addActivity(`service task ${id} closed`);
					changed = true;
				}
			} else {
				this.tasks.delete(id);
				this.addActivity(`service task ${id} disappeared`);
				changed = true;
			}
		}
		this.serviceTaskIds = nextIds;
		return changed;
	}

	private observeServiceTerminals(terminals: WanixServiceTerminal[]): boolean {
		let changed = false;
		const nextIds = new Set(terminals.map((terminal) => terminal.id));
		for (const terminal of terminals) {
			const existing = this.terminals.get(terminal.id);
			if (!existing) {
				this.terminals.set(terminal.id, {
					id: terminal.id,
					label: terminal.label,
					status: "attached",
					serviceObserved: true,
				});
				this.addActivity(`terminal ${terminal.id} observed`);
				changed = true;
				continue;
			}
			if (existing.label !== terminal.label) {
				existing.label = terminal.label;
				changed = true;
			}
			if (existing.status !== "attached") {
				existing.status = "attached";
				changed = true;
			}
			if (!existing.serviceObserved) {
				existing.serviceObserved = true;
				this.addActivity(`terminal ${terminal.id} observed`);
				changed = true;
			}
		}
		for (const id of this.serviceTerminalIds) {
			if (nextIds.has(id)) {
				continue;
			}
			const existing = this.terminals.get(id);
			if (!existing || existing.status === "closed") {
				continue;
			}
			existing.status = "closed";
			this.addActivity(`terminal ${id} closed`);
			changed = true;
		}
		this.serviceTerminalIds = nextIds;
		return changed;
	}

	private refresh(): void {
		this.emitter.fire(undefined);
	}
}

function leaf(
	id: string,
	label: string,
	description?: string,
	icon?: string,
	command?: vscode.Command,
	contextValue?: string,
	metadata: { taskId?: string; sourcePath?: string; outputPath?: string; metadataPath?: string; path?: string; beforePath?: string; afterPath?: string; children?: SystemTreeItem[] } = {},
): SystemTreeItem {
	return {
		type: "leaf",
		id,
		label,
		description,
		icon: icon ? new vscode.ThemeIcon(icon) : undefined,
		command,
		contextValue,
		taskId: metadata.taskId,
		sourcePath: metadata.sourcePath,
		outputPath: metadata.outputPath,
		metadataPath: metadata.metadataPath,
		path: metadata.path,
		beforePath: metadata.beforePath,
		afterPath: metadata.afterPath,
		children: metadata.children,
	};
}

function treeItemTooltip(element: SystemTreeItem & { type: "leaf" }): string {
	return [
		element.label,
		element.description,
		element.path ? `Path: ${element.path}` : undefined,
		element.sourcePath ? `Source: ${element.sourcePath}` : undefined,
		element.outputPath ? `Output: ${element.outputPath}` : undefined,
		element.metadataPath ? `Metadata: ${element.metadataPath}` : undefined,
		element.beforePath ? `Before: ${element.beforePath}` : undefined,
		element.afterPath ? `After: ${element.afterPath}` : undefined,
	].filter((part): part is string => Boolean(part)).join("\n");
}

function taskArtifactItems(task: TaskRecord): SystemTreeItem[] {
	const items: SystemTreeItem[] = [];
	if (task.sourcePath && task.kind === "qjs") {
		items.push(leaf(`task:${task.id}:source`, "Source", pathDescription(task.sourcePath), "go-to-file", {
			command: "workbench.openWanixTaskSource",
			title: "Open Task Source",
			arguments: [task.sourcePath],
		}));
	}
	if (task.outputPath) {
		items.push(leaf(`task:${task.id}:output`, "Transcript", pathDescription(task.outputPath), "output", {
			command: "workbench.openWanixTaskOutput",
			title: "Open Task Output",
			arguments: [task.outputPath],
		}));
	}
	if (task.metadataPath) {
		items.push(leaf(`task:${task.id}:metadata`, "Metadata", pathDescription(task.metadataPath), "json", {
			command: "workbench.openWanixTaskMetadata",
			title: "Open Task Metadata",
			arguments: [task.metadataPath],
		}));
	}
	items.push(leaf(`task:${task.id}:terminal`, "Terminal", "focus output", "terminal", {
		command: "workbench.focusTaskTerminal",
		title: "Focus Task Terminal",
		arguments: [{ taskId: task.id }],
	}));
	items.push(leaf(`task:${task.id}:service`, `#task/${task.id}`, "service dir", "server", {
		command: "workbench.openWanixPath",
		title: "Open Wanix Path",
		arguments: [`#task/${task.id}`],
	}));
	return items;
}

function routeRunArtifactItems(run: RouteRunRecord): SystemTreeItem[] {
	const items: SystemTreeItem[] = [];
	if (run.previewPath) {
		items.push(leaf(`route-run:${run.id}:response`, "Response Report", pathDescription(run.previewPath), "output", {
			command: "workbench.openWanixPath",
			title: "Open Wanix Path",
			arguments: [run.previewPath],
		}));
	}
	if (run.sourcePath) {
		items.push(leaf(`route-run:${run.id}:source`, "Handler Source", pathDescription(run.sourcePath), "go-to-file", {
			command: "workbench.openWanixPath",
			title: "Open Wanix Path",
			arguments: [run.sourcePath],
		}));
	}
	for (const [index, artifact] of (run.artifacts || []).entries()) {
		items.push(leaf(`route-run:${run.id}:artifact:${index}`, artifact.label, pathDescription(artifact.path), artifact.icon || "file", {
			command: "workbench.openWanixPath",
			title: "Open Wanix Path",
			arguments: [artifact.path],
		}));
	}
	if (run.url) {
		items.push(leaf(`route-run:${run.id}:url`, "URL", run.url, "link-external"));
	}
	return items;
}

function activityItem(entry: ActivityRecord): SystemTreeItem {
	const paths = uniquePaths(entry.paths || (entry.path ? [entry.path] : []));
	const path = entry.path || paths[paths.length - 1];
	const hasMultiplePaths = paths.length > 1;
	const children = paths.length > 1
		? paths.map((candidate, index) => leaf(
			`activity:${entry.id}:path:${index}`,
			candidate,
			candidate === path ? "open target" : "touched path",
			candidate === path ? "go-to-file" : "file",
			candidate === path ? {
				command: "workbench.openWanixPath",
				title: "Open Wanix Path",
				arguments: [candidate],
			} : undefined,
			candidate === path ? "wanixActivityPath" : undefined,
			{ path: candidate },
		))
		: undefined;
	return leaf(
		`activity:${entry.id}`,
		entry.label,
		path ? pathDescription(path) : undefined,
		"history",
		path && !hasMultiplePaths ? {
			command: "workbench.openWanixPath",
			title: "Open Wanix Path",
			arguments: [path],
		} : undefined,
		path ? "wanixActivityPath" : undefined,
		{ path, children },
	);
}

function routeRunDescription(run: RouteRunRecord): string {
	return run.url ? `${run.status} · ${run.url}` : run.status;
}

function pathDescription(path: string): string {
	const normalized = path.endsWith("/") && path.length > 1 ? path.slice(0, -1) : path;
	const slash = normalized.lastIndexOf("/");
	return slash >= 0 ? normalized.slice(slash + 1) : normalized;
}

function removeMapEntries<K, V>(map: Map<K, V>, shouldRemove: (value: V) => boolean): number {
	let removed = 0;
	for (const [key, value] of map) {
		if (shouldRemove(value)) {
			map.delete(key);
			removed += 1;
		}
	}
	return removed;
}

function formatClearCount(count: number, noun: string): string {
	return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

function uniquePaths(paths: string[] | undefined): string[] {
	return [...new Set((paths || []).filter((path) => path.length > 0))];
}

function samePaths(left: string[] | undefined, right: string[]): boolean {
	const normalizedLeft = uniquePaths(left);
	if (normalizedLeft.length !== right.length) {
		return false;
	}
	return normalizedLeft.every((path, index) => path === right[index]);
}

function actionLeaf(
	id: string,
	label: string,
	description: string,
	icon: string,
	command: string,
): SystemTreeItem {
	return leaf(id, label, description, icon, {
		command,
		title: label,
	});
}

function categoryIcon(id: CategoryId): vscode.ThemeIcon {
	switch (id) {
		case "actions":
			return new vscode.ThemeIcon("run-all");
		case "drivers":
			return new vscode.ThemeIcon("symbol-method");
		case "tasks":
			return new vscode.ThemeIcon("server-process");
		case "terminals":
			return new vscode.ThemeIcon("terminal");
		case "namespace":
			return new vscode.ThemeIcon("root-folder");
		case "routes":
			return new vscode.ThemeIcon("globe");
		case "routeRuns":
			return new vscode.ThemeIcon("globe");
		case "agent":
			return new vscode.ThemeIcon("tools");
		case "activity":
			return new vscode.ThemeIcon("history");
	}
}

function routeRecords(config: WanixSystemConfig): RouteRecord[] {
	const routes: RouteRecord[] = [];
	const route = config.httpApp;
	if (route?.route && route.status !== "disabled") {
		routes.push({
			id: "http-app",
			label: route.route,
			description: route.source || route.protocol || "http app",
			protocol: route.protocol || "wanix-http-app.v1",
			command: {
				command: "workbench.openHttpAppDemo",
				title: "Preview HTTP App Demo",
			},
		});
	}
	if (config.v86?.launchUrl) {
		routes.push({
			id: "direct-v86",
			label: "direct-v86",
			description: v86RouteDescription(config.v86),
			protocol: "wanix-direct-v86.v1",
			contextValue: "wanixDirectV86Route",
			command: {
				command: "workbench.openDirectV86",
				title: "Open direct-v86 VM",
			},
		});
	}
	return routes;
}

function routeDescription(route: RouteRecord): string {
	if (!route.previewStatus) {
		return route.description;
	}
	return `${route.description} · last ${route.previewStatus}`;
}

function formatHttpStatus(status: number, statusText?: string): string {
	const suffix = statusText?.trim();
	return suffix ? `${status} ${suffix}` : String(status);
}

function v86RouteDescription(v86: NonNullable<WanixSystemConfig["v86"]>): string {
	if (v86.boot?.ready) {
		return "boot root ready";
	}
	const missing = v86.boot?.missing?.length ? `missing ${v86.boot.missing.join(", ")}` : undefined;
	const rootfsStatus = v86.rootfs?.status ? `rootfs ${v86.rootfs.status}` : "boot root unprepared";
	return missing || rootfsStatus;
}

function namespaceIcon(path: string): string {
	if (path.startsWith("#")) {
		return "server";
	}
	return "root-folder";
}

function taskIcon(status: TaskStatus): string {
	switch (status) {
		case "running":
			return "loading";
		case "exited":
			return "pass";
		case "closed":
			return "circle-slash";
	}
}

function taskDisplayName(task: Pick<TaskRecord, "kind" | "label">): string {
	return task.label === task.kind ? task.kind : `${task.kind} ${task.label}`;
}

function formatExitCode(code: number | undefined): string {
	return typeof code === "number" ? String(code) : "?";
}

function shouldReplaceServiceKind(existingKind: string, serviceKind: string): boolean {
	return Boolean(serviceKind) && (existingKind === "" || existingKind === "task" || existingKind === "auto");
}

function shouldReplaceServiceLabel(existingLabel: string, kind: string, serviceLabel: string): boolean {
	if (!serviceLabel || existingLabel === serviceLabel || isPlaceholderServiceLabel(serviceLabel)) {
		return false;
	}
	return existingLabel === kind || existingLabel === "task" || existingLabel.startsWith(`${kind} `);
}

function isPlaceholderServiceLabel(label: string): boolean {
	return label === "noop" || label === "task" || label === "auto";
}
