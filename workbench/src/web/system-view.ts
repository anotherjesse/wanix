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
};

type TerminalRecord = {
	id: string;
	label: string;
	status: TerminalStatus;
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
};

type ActivityRecord = {
	id: number;
	label: string;
};

type CategoryId = "drivers" | "tasks" | "terminals" | "namespace" | "routes" | "activity";

type SystemTreeItem =
	| { type: "category"; id: CategoryId; label: string }
	| { type: "leaf"; id: string; label: string; description?: string; icon?: vscode.ThemeIcon; command?: vscode.Command; contextValue?: string; taskId?: string; sourcePath?: string; outputPath?: string; path?: string };

const CATEGORIES: Array<SystemTreeItem & { type: "category" }> = [
	{ type: "category", id: "drivers", label: "Drivers" },
	{ type: "category", id: "tasks", label: "Tasks" },
	{ type: "category", id: "terminals", label: "Terminals" },
	{ type: "category", id: "namespace", label: "Namespace" },
	{ type: "category", id: "routes", label: "Routes" },
	{ type: "category", id: "activity", label: "Activity" },
];

export class WanixSystemView implements vscode.TreeDataProvider<SystemTreeItem>, vscode.Disposable {
	private readonly emitter = new vscode.EventEmitter<SystemTreeItem | undefined>();
	private readonly disposables: vscode.Disposable[] = [this.emitter];
	private drivers: string[] = [];
	private tasks = new Map<string, TaskRecord>();
	private terminals = new Map<string, TerminalRecord>();
	private namespace: NamespaceRecord[] = [];
	private routes: RouteRecord[] = [];
	private activity: ActivityRecord[] = [];
	private nextActivityId = 1;

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
		this.routes = routeRecords(config);
		if (this.routes.length > 0) {
			this.addActivity("http app route discovered");
		}
		this.refresh();
	}

	register(context: vscode.ExtensionContext): void {
		this.disposables.push(vscode.window.registerTreeDataProvider("wanix.system", this));
		context.subscriptions.push(this);
	}

	taskStarted(id: string, kind: string, label: string, options: { sourcePath?: string; outputPath?: string } = {}): void {
		this.tasks.set(id, { id, kind, label, status: "running", sourcePath: options.sourcePath, outputPath: options.outputPath });
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
		this.terminals.set(id, { id, label, status: "attached" });
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

	filesystemActivity(label = "filesystem activity"): void {
		this.addActivity(label);
		this.refresh();
	}

	getTreeItem(element: SystemTreeItem): vscode.TreeItem {
		if (element.type === "category") {
			const item = new vscode.TreeItem(element.label, vscode.TreeItemCollapsibleState.Expanded);
			item.iconPath = categoryIcon(element.id);
			return item;
		}
		const item = new vscode.TreeItem(element.label, vscode.TreeItemCollapsibleState.None);
		item.description = element.description;
		item.iconPath = element.icon;
		item.command = element.command;
		item.contextValue = element.contextValue;
		return item;
	}

	getChildren(element?: SystemTreeItem): vscode.ProviderResult<SystemTreeItem[]> {
		if (!element) {
			return CATEGORIES;
		}
		if (element.type !== "category") {
			return [];
		}
		switch (element.id) {
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
					? this.routes.map((route) => leaf(`route:${route.id}`, route.label, route.description, "globe", {
						command: "workbench.openHttpAppDemo",
						title: "Preview HTTP App Demo",
					}, "wanixHttpRoute"))
					: [leaf("routes:empty", "no routes advertised")];
			case "activity":
				return this.activity.length > 0
					? this.activity.map((entry) => leaf(`activity:${entry.id}`, entry.label, undefined, "history"))
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
			const description = task.status === "exited"
				? `exited ${formatExitCode(task.exitCode)}`
				: task.status;
			const command = task.sourcePath ? {
				command: "workbench.openWanixTaskSource",
				title: "Open Task Source",
				arguments: [task.sourcePath],
			} : undefined;
			const contextValue = task.sourcePath || task.outputPath ? "wanixTaskWithArtifacts" : "wanixTask";
			return leaf(`task:${task.id}`, `${task.id} ${taskDisplayName(task)}`, description, taskIcon(task.status), command, contextValue, {
				taskId: task.id,
				sourcePath: task.sourcePath,
				outputPath: task.outputPath,
				path: `#task/${task.id}`,
			});
		});
	}

	private terminalItems(): SystemTreeItem[] {
		const terminals = [...this.terminals.values()].sort((left, right) => Number(left.id) - Number(right.id));
		if (terminals.length === 0) {
			return [leaf("terminals:empty", "no observed terminals")];
		}
		return terminals.map((terminal) => {
			const label = terminal.id === "shell" ? terminal.label : `${terminal.id} ${terminal.label}`;
			const path = terminal.id === "shell" ? undefined : `#term/${terminal.id}`;
			const contextValue = path ? "wanixTerminalService" : undefined;
			return leaf(`terminal:${terminal.id}`, label, terminal.status, "terminal", undefined, contextValue, { path });
		});
	}

	private addActivity(label: string): void {
		if (this.activity[0]?.label === label) {
			return;
		}
		this.activity.unshift({ id: this.nextActivityId++, label });
		this.activity = this.activity.slice(0, 12);
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
	metadata: { taskId?: string; sourcePath?: string; outputPath?: string; path?: string } = {},
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
		path: metadata.path,
	};
}

function categoryIcon(id: CategoryId): vscode.ThemeIcon {
	switch (id) {
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
		case "activity":
			return new vscode.ThemeIcon("history");
	}
}

function routeRecords(config: WanixSystemConfig): RouteRecord[] {
	const route = config.httpApp;
	if (!route?.route || route.status === "disabled") {
		return [];
	}
	return [{
		id: "http-app",
		label: route.route,
		description: route.source || route.protocol || "http app",
		protocol: route.protocol || "wanix-http-app.v1",
	}];
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
