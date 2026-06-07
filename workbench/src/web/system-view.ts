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
	name?: string;
	runtime?: string;
	sourcePath?: string;
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
	description?: string;
	evidence?: string;
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

type TourStatus = "running" | "ok" | "failed" | "report";

type TourRecord = {
	id: number;
	label: string;
	status: TourStatus;
	description?: string;
	artifacts?: string[];
	path?: string;
	error?: string;
};

type CheckStatus = "running" | "ok" | "warn" | "failed" | "report";

type CheckRecord = {
	id: number;
	label: string;
	status: CheckStatus;
	description?: string;
	artifacts?: string[];
	path?: string;
	error?: string;
};

type ReportRecord = {
	id: number;
	label: string;
	path: string;
	kind?: string;
	description?: string;
	icon?: string;
	artifacts?: string[];
};

type DataStoreRecord = {
	id: number;
	label: string;
	path: string;
	kind?: string;
	description?: string;
	sourcePath?: string;
	routeLabel?: string;
	artifacts?: string[];
};

type CategoryId = "actions" | "tour" | "checks" | "reports" | "dataStores" | "drivers" | "tasks" | "terminals" | "namespace" | "routes" | "routeRuns" | "agent" | "activity";

type SystemTreeItem =
	| { type: "category"; id: CategoryId; label: string }
	| { type: "leaf"; id: string; label: string; description?: string; icon?: vscode.ThemeIcon; command?: vscode.Command; contextValue?: string; taskId?: string; sourcePath?: string; outputPath?: string; metadataPath?: string; path?: string; beforePath?: string; afterPath?: string; children?: SystemTreeItem[] };

const CATEGORIES: Array<SystemTreeItem & { type: "category" }> = [
	{ type: "category", id: "actions", label: "Actions" },
	{ type: "category", id: "tour", label: "Tour" },
	{ type: "category", id: "reports", label: "Reports" },
	{ type: "category", id: "dataStores", label: "Data Stores" },
	{ type: "category", id: "checks", label: "Checks" },
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
	private tour: TourRecord[] = [];
	private checks: CheckRecord[] = [];
	private reports: ReportRecord[] = [];
	private dataStores: DataStoreRecord[] = [];
	private agent: AgentRecord[] = [];
	private activity: ActivityRecord[] = [];
	private serviceTaskIds = new Set<string>();
	private serviceTerminalIds = new Set<string>();
	private hasHttpApp = false;
	private hasV86 = false;
	private nextActivityId = 1;
	private nextRouteRunId = 1;
	private nextTourId = 1;
	private nextCheckId = 1;
	private nextReportId = 1;
	private nextDataStoreId = 1;
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

	filesystemActivity(label = "filesystem activity", options: { description?: string; evidence?: string; path?: string; paths?: string[] } = {}): void {
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

	tourStarted(label: string): void {
		this.tour = [];
		this.nextTourId = 1;
		this.upsertTourStep(label, "running", { description: "full cockpit demo arc" });
		this.addActivity(`tour ${label} started`);
		this.refresh();
	}

	tourStepStarted(label: string, options: { description?: string; artifacts?: string[] } = {}): void {
		this.upsertTourStep(label, "running", options);
		this.addActivity(`tour ${label} started`);
		this.refresh();
	}

	tourStepCompleted(label: string): void {
		this.upsertTourStep(label, "ok");
		this.addActivity(`tour ${label} completed`);
		this.refresh();
	}

	tourStepFailed(label: string, error: unknown): void {
		this.upsertTourStep(label, "failed", {
			error: error instanceof Error ? error.message : String(error),
		});
		this.addActivity(`tour ${label} failed`);
		this.refresh();
	}

	tourReport(path: string, status: "complete" | "failed"): void {
		this.tour.unshift({
			id: this.nextTourId++,
			label: "Tour Report",
			status: "report",
			description: status,
			path,
			artifacts: [path],
		});
		this.addActivity("tour report written", { path });
		this.refresh();
	}

	checksStarted(label: string): void {
		this.checks = [];
		this.nextCheckId = 1;
		this.upsertCheck(label, "running", { description: "browser cockpit self-check" });
		this.addActivity(`self-check ${label} started`);
		this.refresh();
	}

	checkStarted(label: string, options: { description?: string; artifacts?: string[] } = {}): void {
		this.upsertCheck(label, "running", options);
		this.addActivity(`self-check ${label} started`);
		this.refresh();
	}

	checkPassed(label: string, options: { description?: string; artifacts?: string[] } = {}): void {
		this.upsertCheck(label, "ok", options);
		this.addActivity(`self-check ${label} ok`);
		this.refresh();
	}

	checkWarned(label: string, options: { description?: string; artifacts?: string[] } = {}): void {
		this.upsertCheck(label, "warn", options);
		this.addActivity(`self-check ${label} warning`);
		this.refresh();
	}

	checkFailed(label: string, error: unknown, options: { description?: string; artifacts?: string[] } = {}): void {
		this.upsertCheck(label, "failed", {
			...options,
			error: error instanceof Error ? error.message : String(error),
		});
		this.addActivity(`self-check ${label} failed`);
		this.refresh();
	}

	checkReport(path: string, status: "ok" | "warn" | "failed", artifacts: string[] = [path]): void {
		const paths = uniquePaths([path, ...artifacts]);
		this.checks.unshift({
			id: this.nextCheckId++,
			label: "Self Check Report",
			status: "report",
			description: status,
			path,
			artifacts: paths,
		});
		this.addActivity("self-check report written", { path, paths });
		this.refresh();
	}

	reportPublished(label: string, path: string, options: { kind?: string; description?: string; icon?: string; artifacts?: string[] } = {}): void {
		const artifacts = uniquePaths([path, ...(options.artifacts || [])]);
		const existing = this.reports.find((entry) => entry.path === path);
		if (existing) {
			existing.label = label;
			existing.kind = options.kind || existing.kind;
			existing.description = options.description || existing.description;
			existing.icon = options.icon || existing.icon;
			existing.artifacts = artifacts;
		} else {
			this.reports.unshift({
				id: this.nextReportId++,
				label,
				path,
				kind: options.kind,
				description: options.description,
				icon: options.icon,
				artifacts,
			});
			this.reports = this.reports.slice(0, 12);
		}
		this.addActivity(`report ${label} published`, { path, paths: artifacts });
		this.refresh();
	}

	httpAppCatalogPublished(entries: Array<{ name: string; runtime: string; sourcePath: string; routeLabel?: string; previewPath?: string }>): void {
		const ids = new Set(entries.map((entry) => httpAppRouteId(entry.name)));
		this.routes = this.routes.filter((route) => !isIndexedHttpAppRoute(route) || ids.has(route.id));
		for (const entry of entries) {
			this.upsertHttpAppRoute(entry);
		}
		this.addActivity(entries.length > 0
			? `http app catalog indexed ${formatClearCount(entries.length, "app")}`
			: "http app catalog found no apps");
		this.refresh();
	}

	httpAppIndexed(entry: { name: string; runtime: string; sourcePath: string; routeLabel?: string; previewPath?: string }): void {
		this.upsertHttpAppRoute(entry);
		this.addActivity(`http app ${entry.name} indexed`, { path: entry.sourcePath });
		this.refresh();
	}

	reportInventoryMarkdown(options: { generatedAt?: Date; markdownPath?: string; jsonPath?: string } = {}): string {
		const generatedAt = options.generatedAt || new Date();
		const byKind = new Map<string, ReportRecord[]>();
		for (const report of this.reports) {
			const kind = report.kind || "report";
			const entries = byKind.get(kind) || [];
			entries.push(report);
			byKind.set(kind, entries);
		}
		return [
			"# Wanix Cockpit Reports",
			"",
			`Generated: ${generatedAt.toISOString()}`,
			"Schema: wanix.cockpit-reports.v1",
			options.jsonPath ? `JSON: ${displayJournalPath(options.jsonPath)}` : undefined,
			"",
			"## Reports",
			"",
			...Array.from(byKind.entries()).flatMap(([kind, reports]) => [
				`### ${kind}`,
				"",
				...reports.flatMap(reportInventoryMarkdownLines),
			]),
		].filter((line): line is string => line !== undefined).join("\n");
	}

	reportInventoryJson(options: { generatedAt?: Date; markdownPath?: string; jsonPath?: string } = {}): string {
		const generatedAt = options.generatedAt || new Date();
		const reports = this.reports.map(reportInventorySnapshot);
		return `${JSON.stringify({
			schema: "wanix.cockpit-reports.v1",
			generatedAt: generatedAt.toISOString(),
			markdownPath: options.markdownPath ? displayJournalPath(options.markdownPath) : undefined,
			jsonPath: options.jsonPath ? displayJournalPath(options.jsonPath) : undefined,
			reportCount: reports.length,
			artifactPaths: uniquePaths(this.reports.flatMap((report) => report.artifacts || [report.path])).map(displayJournalPath),
			reports,
		}, null, 2)}\n`;
	}

	dataStorePublished(label: string, path: string, options: { kind?: string; description?: string; sourcePath?: string; routeLabel?: string; artifacts?: string[] } = {}): void {
		const artifacts = uniquePaths([path, ...(options.artifacts || [])]);
		const existing = this.dataStores.find((entry) => entry.path === path);
		if (existing) {
			existing.label = label;
			existing.kind = options.kind || existing.kind;
			existing.description = options.description || existing.description;
			existing.sourcePath = options.sourcePath || existing.sourcePath;
			existing.routeLabel = options.routeLabel || existing.routeLabel;
			existing.artifacts = artifacts;
		} else {
			this.dataStores.unshift({
				id: this.nextDataStoreId++,
				label,
				path,
				kind: options.kind,
				description: options.description,
				sourcePath: options.sourcePath,
				routeLabel: options.routeLabel,
				artifacts,
			});
			this.dataStores = this.dataStores.slice(0, 12);
		}
		this.addActivity(`data store ${label} indexed`, { path, paths: artifacts });
		this.refresh();
	}

	dataStoreInventoryMarkdown(options: { generatedAt?: Date; markdownPath?: string; jsonPath?: string } = {}): string {
		const generatedAt = options.generatedAt || new Date();
		const byKind = new Map<string, DataStoreRecord[]>();
		for (const store of this.dataStores) {
			const kind = store.kind || "store";
			const entries = byKind.get(kind) || [];
			entries.push(store);
			byKind.set(kind, entries);
		}
		return [
			"# Wanix Data Stores",
			"",
			`Generated: ${generatedAt.toISOString()}`,
			"Schema: wanix.data-stores.v1",
			options.jsonPath ? `JSON: ${displayJournalPath(options.jsonPath)}` : undefined,
			"",
			"## Stores",
			"",
			...Array.from(byKind.entries()).flatMap(([kind, stores]) => [
				`### ${kind}`,
				"",
				...stores.flatMap(dataStoreInventoryMarkdownLines),
			]),
		].filter((line): line is string => line !== undefined).join("\n");
	}

	dataStoreInventoryJson(options: { generatedAt?: Date; markdownPath?: string; jsonPath?: string } = {}): string {
		const generatedAt = options.generatedAt || new Date();
		const stores = this.dataStores.map(dataStoreInventorySnapshot);
		return `${JSON.stringify({
			schema: "wanix.data-stores.v1",
			generatedAt: generatedAt.toISOString(),
			markdownPath: options.markdownPath ? displayJournalPath(options.markdownPath) : undefined,
			jsonPath: options.jsonPath ? displayJournalPath(options.jsonPath) : undefined,
			storeCount: stores.length,
			artifactPaths: uniquePaths(this.dataStores.flatMap((store) => store.artifacts || [store.path])).map(displayJournalPath),
			stores,
		}, null, 2)}\n`;
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
		if (isIndexedHttpAppRoute(route)) {
			route.contextValue = "wanixHttpCatalogRouteWithPreview";
		}
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

	systemJournalMarkdown(options: { generatedAt?: Date; path?: string; statePath?: string } = {}): string {
		const generatedAt = options.generatedAt || new Date();
		return [
			"# Wanix System Journal",
			"",
			`Generated: ${generatedAt.toISOString()}`,
			options.path ? `Path: ${displayJournalPath(options.path)}` : undefined,
			options.statePath ? `State JSON: ${displayJournalPath(options.statePath)}` : undefined,
			"",
			"## Drivers",
			...journalList(this.drivers.map((driver) => `- ${driver}`)),
			"",
			"## Namespace",
			...journalList(this.namespace.map((entry) => `- ${displayJournalPath(entry.path)} - ${entry.label}`)),
			"",
			"## Tasks",
			...journalList([...this.tasks.values()]
				.sort((left, right) => Number(left.id) - Number(right.id))
				.flatMap(taskJournalLines)),
			"",
			"## Terminals",
			...journalList([...this.terminals.values()]
				.sort((left, right) => Number(left.id) - Number(right.id))
				.map(terminalJournalLine)),
			"",
			"## Routes",
			...journalList(this.routes.flatMap(routeJournalLines)),
			"",
			"## Route Runs",
			...journalList(this.routeRuns.flatMap(routeRunJournalLines)),
			"",
			"## Data Stores",
			...journalList(this.dataStores.flatMap(dataStoreJournalLines)),
			"",
			"## Tour",
			...journalList(this.tour.flatMap(tourJournalLines)),
			"",
			"## Checks",
			...journalList(this.checks.flatMap(checkJournalLines)),
			"",
			"## Reports",
			...journalList(this.reports.flatMap(reportJournalLines)),
			"",
			"## Agent",
			...journalList(this.agent.flatMap(agentJournalLines)),
			"",
			"## Recent Activity",
			...journalList(this.activity.flatMap(activityJournalLines)),
			"",
		].filter((line): line is string => line !== undefined).join("\n");
	}

	systemStateJson(options: { generatedAt?: Date; journalPath?: string; statePath?: string } = {}): string {
		const generatedAt = options.generatedAt || new Date();
		const state = {
			schema: "wanix.system-state.v1",
			generatedAt: generatedAt.toISOString(),
			journalPath: options.journalPath ? displayJournalPath(options.journalPath) : undefined,
			statePath: options.statePath ? displayJournalPath(options.statePath) : undefined,
			drivers: [...this.drivers],
			namespace: this.namespace.map((entry) => ({
				path: displayJournalPath(entry.path),
				label: entry.label,
			})),
			tasks: [...this.tasks.values()]
				.sort((left, right) => Number(left.id) - Number(right.id))
				.map(taskSnapshot),
			terminals: [...this.terminals.values()]
				.sort((left, right) => Number(left.id) - Number(right.id))
				.map(terminalSnapshot),
			routes: this.routes.map(routeSnapshot),
			routeRuns: this.routeRuns.map(routeRunSnapshot),
			dataStores: this.dataStores.map(dataStoreSnapshot),
			tour: this.tour.map(tourSnapshot),
			checks: this.checks.map(checkSnapshot),
			reports: this.reports.map(reportSnapshot),
			agent: this.agent.map(agentSnapshot),
			activity: this.activity.map(activitySnapshot),
		};
		return `${JSON.stringify(state, null, 2)}\n`;
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
			case "tour":
				return this.tourItems();
			case "checks":
				return this.checkItems();
			case "reports":
				return this.reportItems();
			case "dataStores":
				return this.dataStoreItems();
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
						{ path: route.previewPath, sourcePath: route.sourcePath },
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

	private tourItems(): SystemTreeItem[] {
		if (this.tour.length === 0) {
			return [leaf("tour:empty", "no tour has run yet")];
		}
		return this.tour.map((entry) => leaf(
			`tour:${entry.id}`,
			entry.label,
			tourDescription(entry),
			tourIcon(entry.status),
			entry.path ? {
				command: "workbench.openWanixPath",
				title: "Open Wanix Path",
				arguments: [entry.path],
			} : undefined,
			entry.path ? "wanixTourArtifact" : undefined,
			{
				path: entry.path,
				children: tourArtifactItems(entry),
			},
		));
	}

	private checkItems(): SystemTreeItem[] {
		if (this.checks.length === 0) {
			return [leaf("checks:empty", "no self-check has run yet")];
		}
		return this.checks.map((entry) => leaf(
			`check:${entry.id}`,
			entry.label,
			checkDescription(entry),
			checkIcon(entry.status),
			entry.path ? {
				command: "workbench.openWanixPath",
				title: "Open Wanix Path",
				arguments: [entry.path],
			} : undefined,
			entry.path ? "wanixCheckArtifact" : undefined,
			{
				path: entry.path,
				children: checkArtifactItems(entry),
			},
		));
	}

	private reportItems(): SystemTreeItem[] {
		if (this.reports.length === 0) {
			return [leaf("reports:empty", "no reports published yet")];
		}
		return this.reports.map((entry) => leaf(
			`report:${entry.id}`,
			entry.label,
			reportDescription(entry),
			entry.icon || "notebook",
			{
				command: "workbench.openWanixPath",
				title: "Open Wanix Path",
				arguments: [entry.path],
			},
			"wanixReportArtifact",
			{
				path: entry.path,
				children: reportArtifactItems(entry),
			},
		));
	}

	private dataStoreItems(): SystemTreeItem[] {
		if (this.dataStores.length === 0) {
			return [leaf("data-stores:empty", "no data stores indexed yet")];
		}
		return this.dataStores.map((entry) => leaf(
			`data-store:${entry.id}`,
			entry.label,
			dataStoreDescription(entry),
			"database",
			{
				command: "workbench.openWanixPath",
				title: "Open Wanix Path",
				arguments: [entry.path],
			},
			"wanixDataStore",
			{
				path: entry.path,
				sourcePath: entry.sourcePath,
				children: dataStoreArtifactItems(entry),
			},
		));
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
			actionLeaf("action:run-cockpit-tour", "Run OS Cockpit Tour", "full demo", "run-all", "workbench.runCockpitTour"),
			actionLeaf("action:run-self-check", "Run Cockpit Self Check", "diagnose", "checklist", "workbench.runCockpitSelfCheck"),
			actionLeaf("action:prepare-cockpit", "Prepare Cockpit Reports", "make ready", "check-all", "workbench.prepareCockpitReports"),
			actionLeaf("action:open-reports", "Open Report Inventory", "reports", "notebook", "workbench.openCockpitReports"),
			actionLeaf("action:open-data-stores", "Open Data Store Index", "state", "database", "workbench.openDataStoreInventory"),
			actionLeaf("action:open-shell-history", "Open Shell Command History", "shell", "terminal", "workbench.openShellCommandHistory"),
			actionLeaf("action:system-journal", "Open System Journal", "snapshot", "notebook", "workbench.openSystemJournal"),
			actionLeaf("action:agent-tools", "Open Agent Tool Contract", "agent tools", "symbol-method", "workbench.openAgentToolContract"),
			actionLeaf("action:install-agent-repair", "Install Agent Repair Demo", "agent", "bug", "workbench.installAgentRepairDemo"),
			actionLeaf("action:run-agent-repair", "Run Agent Repair Demo", "one click", "tools", "workbench.runAgentRepairDemo"),
			actionLeaf("action:fix-agent-repair", "Fix Current Wanix Program", "current file", "tools", "workbench.fixCurrentWanixProgram"),
		];
		if (this.hasV86) {
			items.splice(4, 0,
				actionLeaf("action:v86-shared", "Open v86 Shared Files Demo", "linux", "vm", "workbench.openV86SharedDemo"),
				actionLeaf("action:direct-v86", "Open direct-v86 VM", "browser", "vm-running", "workbench.openDirectV86"),
			);
		}
		if (this.hasHttpApp) {
			items.splice(4, 0,
				actionLeaf("action:new-http-app", "New HTTP App", "apps", "new-file", "workbench.newHttpApp"),
				actionLeaf("action:preview-current-http-app", "Preview Current HTTP App", "active editor", "globe", "workbench.previewCurrentHttpApp"),
				actionLeaf("action:preview-http", "Preview HTTP App Demo", "http", "globe", "workbench.openHttpAppDemo"),
				actionLeaf("action:preview-http-counter", "Preview HTTP Counter Demo", "stateful http", "server-process", "workbench.openHttpCounterDemo"),
				actionLeaf("action:preview-http-wasm", "Preview HTTP WASM Demo", "wasm http", "server-process", "workbench.openHttpWasmDemo"),
				actionLeaf("action:open-http-catalog", "Open HTTP App Catalog", "apps", "globe", "workbench.openHttpAppCatalog"),
				actionLeaf("action:open-http-handler", "Open HTTP App Handler", "http", "go-to-file", "workbench.openHttpAppHandler"),
			);
		}
		items.push(actionLeaf("action:clear-finished", "Clear Finished Rows", "sidebar", "clear-all", "workbench.clearFinishedSystemRows"));
		return items;
	}

	private addActivity(label: string, options: { description?: string; evidence?: string; path?: string; paths?: string[] } = {}): void {
		const paths = uniquePaths(options.paths || (options.path ? [options.path] : []));
		const path = options.path || paths[paths.length - 1];
		const description = options.description;
		const evidence = options.evidence;
		if (this.activity[0]?.label === label && this.activity[0].description === description && this.activity[0].evidence === evidence && samePaths(this.activity[0].paths, paths) && this.activity[0].path === path) {
			return;
		}
		this.activity.unshift({ id: this.nextActivityId++, label, description, evidence, path, paths });
		this.activity = this.activity.slice(0, 12);
	}

	private upsertTourStep(label: string, status: TourStatus, options: { description?: string; artifacts?: string[]; error?: string } = {}): void {
		const existing = this.tour.find((entry) => entry.label === label && entry.status !== "report");
		if (existing) {
			existing.status = status;
			existing.description = options.description || existing.description;
			existing.artifacts = options.artifacts || existing.artifacts;
			existing.error = options.error || existing.error;
			return;
		}
		this.tour.push({
			id: this.nextTourId++,
			label,
			status,
			description: options.description,
			artifacts: options.artifacts,
			error: options.error,
		});
	}

	private upsertCheck(label: string, status: CheckStatus, options: { description?: string; artifacts?: string[]; error?: string } = {}): void {
		const existing = this.checks.find((entry) => entry.label === label && entry.status !== "report");
		if (existing) {
			existing.status = status;
			existing.description = options.description || existing.description;
			existing.artifacts = options.artifacts || existing.artifacts;
			existing.error = options.error || existing.error;
			return;
		}
		this.checks.push({
			id: this.nextCheckId++,
			label,
			status,
			description: options.description,
			artifacts: options.artifacts,
			error: options.error,
		});
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

	private upsertHttpAppRoute(entry: { name: string; runtime: string; sourcePath: string; routeLabel?: string; previewPath?: string }): void {
		const id = httpAppRouteId(entry.name);
		const label = entry.routeLabel || `/.wanix/app/${entry.name}`;
		const description = `${entry.runtime} handler · ${displayJournalPath(entry.sourcePath)}`;
		const command = {
			command: "workbench.previewHttpCatalogApp",
			title: "Preview HTTP App Route",
			arguments: [{ name: entry.name, sourcePath: entry.sourcePath }],
		};
		const existing = this.routes.find((route) => route.id === id);
		if (existing) {
			existing.label = label;
			existing.description = description;
			existing.protocol = "wanix-http-app.v1";
			existing.command = command;
			existing.contextValue = entry.previewPath || existing.previewPath ? "wanixHttpCatalogRouteWithPreview" : "wanixHttpCatalogRoute";
			existing.name = entry.name;
			existing.runtime = entry.runtime;
			existing.sourcePath = entry.sourcePath;
			existing.previewPath = entry.previewPath || existing.previewPath;
			return;
		}
		this.routes.push({
			id,
			label,
			description,
			protocol: "wanix-http-app.v1",
			command,
			contextValue: entry.previewPath ? "wanixHttpCatalogRouteWithPreview" : "wanixHttpCatalogRoute",
			name: entry.name,
			runtime: entry.runtime,
			sourcePath: entry.sourcePath,
			previewPath: entry.previewPath,
		});
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

function tourArtifactItems(entry: TourRecord): SystemTreeItem[] {
	return uniquePaths(entry.artifacts).map((path, index) => leaf(
		`tour:${entry.id}:artifact:${index}`,
		path,
		pathDescription(path),
		path === entry.path ? "output" : "file",
		{
			command: "workbench.openWanixPath",
			title: "Open Wanix Path",
			arguments: [path],
		},
		"wanixTourArtifact",
		{ path },
	));
}

function checkArtifactItems(entry: CheckRecord): SystemTreeItem[] {
	return uniquePaths(entry.artifacts).map((path, index) => leaf(
		`check:${entry.id}:artifact:${index}`,
		path,
		pathDescription(path),
		path === entry.path ? "output" : "file",
		{
			command: "workbench.openWanixPath",
			title: "Open Wanix Path",
			arguments: [path],
		},
		"wanixCheckArtifact",
		{ path },
	));
}

function reportArtifactItems(entry: ReportRecord): SystemTreeItem[] {
	return uniquePaths(entry.artifacts).map((path, index) => leaf(
		`report:${entry.id}:artifact:${index}`,
		path,
		pathDescription(path),
		path === entry.path ? "notebook" : "file",
		{
			command: "workbench.openWanixPath",
			title: "Open Wanix Path",
			arguments: [path],
		},
		"wanixReportArtifact",
		{ path },
	));
}

function dataStoreArtifactItems(entry: DataStoreRecord): SystemTreeItem[] {
	const items: SystemTreeItem[] = [
		leaf(`data-store:${entry.id}:state`, "State File", pathDescription(entry.path), "database", {
			command: "workbench.openWanixPath",
			title: "Open Wanix Path",
			arguments: [entry.path],
		}, "wanixDataStore", { path: entry.path }),
	];
	if (entry.sourcePath) {
		items.push(leaf(`data-store:${entry.id}:source`, "Source", pathDescription(entry.sourcePath), "go-to-file", {
			command: "workbench.openWanixPath",
			title: "Open Wanix Path",
			arguments: [entry.sourcePath],
		}, "wanixDataStore", { path: entry.sourcePath }));
	}
	for (const [index, path] of uniquePaths(entry.artifacts)
		.filter((path) => path !== entry.path && path !== entry.sourcePath)
		.entries()) {
		items.push(leaf(`data-store:${entry.id}:artifact:${index}`, path, pathDescription(path), "file", {
			command: "workbench.openWanixPath",
			title: "Open Wanix Path",
			arguments: [path],
		}, "wanixDataStore", { path }));
	}
	if (entry.routeLabel) {
		items.push(leaf(`data-store:${entry.id}:route`, "Route", entry.routeLabel, "globe"));
	}
	return items;
}

function activityItem(entry: ActivityRecord): SystemTreeItem {
	const paths = uniquePaths(entry.paths || (entry.path ? [entry.path] : []));
	const path = entry.path || paths[paths.length - 1];
	const children: SystemTreeItem[] = [
		...(paths.length > 1
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
		: []),
		...(entry.evidence
			? [leaf(`activity:${entry.id}:evidence`, "Evidence", entry.evidence, "verified")]
			: []),
	];
	return leaf(
		`activity:${entry.id}`,
		entry.label,
		entry.description || (path ? pathDescription(path) : undefined),
		"history",
		path ? {
			command: "workbench.openWanixPath",
			title: "Open Wanix Path",
			arguments: [path],
		} : undefined,
		path ? "wanixActivityPath" : undefined,
		{ path, children: children.length > 0 ? children : undefined },
	);
}

function routeRunDescription(run: RouteRunRecord): string {
	return run.url ? `${run.status} · ${run.url}` : run.status;
}

function tourDescription(entry: TourRecord): string {
	const status = entry.status === "ok" ? "ok" : entry.status;
	const detail = entry.error || entry.description;
	return detail ? `${status} · ${detail}` : status;
}

function checkDescription(entry: CheckRecord): string {
	const status = entry.status === "ok" ? "ok" : entry.status;
	const detail = entry.error || entry.description;
	return detail ? `${status} · ${detail}` : status;
}

function reportDescription(entry: ReportRecord): string {
	const detail = entry.description;
	return detail ? `${detail} · ${pathDescription(entry.path)}` : pathDescription(entry.path);
}

function dataStoreDescription(entry: DataStoreRecord): string {
	const details = [entry.kind, entry.description, entry.routeLabel].filter((part): part is string => Boolean(part));
	return details.length > 0 ? `${details.join(" · ")} · ${pathDescription(entry.path)}` : pathDescription(entry.path);
}

function journalList(lines: string[]): string[] {
	return lines.length > 0 ? lines : ["- none"];
}

function taskSnapshot(task: TaskRecord): object {
	return {
		id: task.id,
		kind: task.kind,
		label: task.label,
		status: task.status,
		exitCode: task.exitCode,
		sourcePath: task.sourcePath ? displayJournalPath(task.sourcePath) : undefined,
		outputPath: task.outputPath ? displayJournalPath(task.outputPath) : undefined,
		metadataPath: task.metadataPath ? displayJournalPath(task.metadataPath) : undefined,
		serviceObserved: Boolean(task.serviceObserved),
		servicePath: `#task/${task.id}`,
	};
}

function taskJournalLines(task: TaskRecord): string[] {
	const status = task.status === "exited" ? `exited ${formatExitCode(task.exitCode)}` : task.status;
	return [
		`- ${task.id} ${taskDisplayName(task)} - ${status}${task.serviceObserved ? " (#task observed)" : ""}`,
		task.sourcePath ? `  - source: ${displayJournalPath(task.sourcePath)}` : undefined,
		task.outputPath ? `  - transcript: ${displayJournalPath(task.outputPath)}` : undefined,
		task.metadataPath ? `  - metadata: ${displayJournalPath(task.metadataPath)}` : undefined,
		`  - service: #task/${task.id}`,
	].filter((line): line is string => line !== undefined);
}

function terminalSnapshot(terminal: TerminalRecord): object {
	return {
		id: terminal.id,
		label: terminal.label,
		status: terminal.status,
		serviceObserved: Boolean(terminal.serviceObserved),
		servicePath: terminal.id === "shell" ? undefined : `#term/${terminal.id}`,
	};
}

function terminalJournalLine(terminal: TerminalRecord): string {
	const label = terminal.id === "shell" ? terminal.label : `${terminal.id} ${terminal.label}`;
	return `- ${label} - ${terminal.status}${terminal.serviceObserved ? " (#term observed)" : ""}`;
}

function routeSnapshot(route: RouteRecord): object {
	return {
		id: route.id,
		label: route.label,
		description: routeDescription(route),
		protocol: route.protocol,
		name: route.name,
		runtime: route.runtime,
		sourcePath: route.sourcePath ? displayJournalPath(route.sourcePath) : undefined,
		previewStatus: route.previewStatus,
		previewPath: route.previewPath ? displayJournalPath(route.previewPath) : undefined,
	};
}

function routeJournalLines(route: RouteRecord): string[] {
	return [
		`- ${route.label} - ${routeDescription(route)}`,
		`  - protocol: ${route.protocol}`,
		route.sourcePath ? `  - source: ${displayJournalPath(route.sourcePath)}` : undefined,
		route.previewPath ? `  - latest preview: ${displayJournalPath(route.previewPath)}` : undefined,
	].filter((line): line is string => line !== undefined);
}

function routeRunSnapshot(run: RouteRunRecord): object {
	return {
		id: run.id,
		routeId: run.routeId,
		label: run.label,
		status: run.status,
		url: run.url,
		sourcePath: run.sourcePath ? displayJournalPath(run.sourcePath) : undefined,
		previewPath: run.previewPath ? displayJournalPath(run.previewPath) : undefined,
		artifacts: (run.artifacts || []).map((artifact) => ({
			label: artifact.label,
			path: displayJournalPath(artifact.path),
			icon: artifact.icon,
		})),
	};
}

function routeRunJournalLines(run: RouteRunRecord): string[] {
	return [
		`- ${run.id} ${run.label} - ${routeRunDescription(run)}`,
		run.url ? `  - url: ${run.url}` : undefined,
		run.previewPath ? `  - response: ${displayJournalPath(run.previewPath)}` : undefined,
		run.sourcePath ? `  - handler: ${displayJournalPath(run.sourcePath)}` : undefined,
		...(run.artifacts || []).map((artifact) => `  - ${artifact.label}: ${displayJournalPath(artifact.path)}`),
	].filter((line): line is string => line !== undefined);
}

function dataStoreSnapshot(entry: DataStoreRecord): object {
	return {
		id: entry.id,
		label: entry.label,
		kind: entry.kind,
		description: entry.description,
		path: displayJournalPath(entry.path),
		sourcePath: entry.sourcePath ? displayJournalPath(entry.sourcePath) : undefined,
		routeLabel: entry.routeLabel,
		artifacts: uniquePaths(entry.artifacts).map(displayJournalPath),
	};
}

function dataStoreJournalLines(entry: DataStoreRecord): string[] {
	return [
		`- ${entry.label}${entry.description ? ` - ${entry.description}` : ""}`,
		entry.kind ? `  - kind: ${entry.kind}` : undefined,
		`  - path: ${displayJournalPath(entry.path)}`,
		entry.sourcePath ? `  - source: ${displayJournalPath(entry.sourcePath)}` : undefined,
		entry.routeLabel ? `  - route: ${entry.routeLabel}` : undefined,
		...uniquePaths(entry.artifacts).map((path) => `  - artifact: ${displayJournalPath(path)}`),
	].filter((line): line is string => line !== undefined);
}

function tourSnapshot(entry: TourRecord): object {
	return {
		id: entry.id,
		label: entry.label,
		status: entry.status,
		description: entry.description,
		error: entry.error,
		path: entry.path ? displayJournalPath(entry.path) : undefined,
		artifacts: uniquePaths(entry.artifacts).map(displayJournalPath),
	};
}

function tourJournalLines(entry: TourRecord): string[] {
	return [
		`- ${entry.label} - ${tourDescription(entry)}`,
		entry.path ? `  - path: ${displayJournalPath(entry.path)}` : undefined,
		...uniquePaths(entry.artifacts).map((path) => `  - artifact: ${displayJournalPath(path)}`),
	].filter((line): line is string => line !== undefined);
}

function checkSnapshot(entry: CheckRecord): object {
	return {
		id: entry.id,
		label: entry.label,
		status: entry.status,
		description: entry.description,
		error: entry.error,
		path: entry.path ? displayJournalPath(entry.path) : undefined,
		artifacts: uniquePaths(entry.artifacts).map(displayJournalPath),
	};
}

function checkJournalLines(entry: CheckRecord): string[] {
	return [
		`- ${entry.label} - ${checkDescription(entry)}`,
		entry.path ? `  - path: ${displayJournalPath(entry.path)}` : undefined,
		...uniquePaths(entry.artifacts).map((path) => `  - artifact: ${displayJournalPath(path)}`),
	].filter((line): line is string => line !== undefined);
}

function reportSnapshot(entry: ReportRecord): object {
	return {
		id: entry.id,
		label: entry.label,
		kind: entry.kind,
		description: entry.description,
		path: displayJournalPath(entry.path),
		artifacts: uniquePaths(entry.artifacts).map(displayJournalPath),
	};
}

function reportJournalLines(entry: ReportRecord): string[] {
	return [
		`- ${entry.label}${entry.description ? ` - ${entry.description}` : ""}`,
		`  - path: ${displayJournalPath(entry.path)}`,
		...uniquePaths(entry.artifacts).map((path) => `  - artifact: ${displayJournalPath(path)}`),
	];
}

function reportInventorySnapshot(entry: ReportRecord): object {
	return {
		id: entry.id,
		label: entry.label,
		kind: entry.kind || "report",
		description: entry.description,
		path: displayJournalPath(entry.path),
		artifacts: uniquePaths(entry.artifacts || [entry.path]).map(displayJournalPath),
	};
}

function reportInventoryMarkdownLines(entry: ReportRecord): string[] {
	return [
		`- ${entry.label}: ${displayJournalPath(entry.path)}${entry.description ? ` - ${entry.description}` : ""}`,
		...uniquePaths(entry.artifacts || [entry.path])
			.filter((path) => path !== entry.path)
			.map((path) => `  - ${displayJournalPath(path)}`),
		"",
	];
}

function dataStoreInventorySnapshot(entry: DataStoreRecord): object {
	return {
		id: entry.id,
		label: entry.label,
		kind: entry.kind || "store",
		description: entry.description,
		path: displayJournalPath(entry.path),
		sourcePath: entry.sourcePath ? displayJournalPath(entry.sourcePath) : undefined,
		routeLabel: entry.routeLabel,
		artifacts: uniquePaths(entry.artifacts || [entry.path]).map(displayJournalPath),
	};
}

function dataStoreInventoryMarkdownLines(entry: DataStoreRecord): string[] {
	return [
		`- ${entry.label}: ${displayJournalPath(entry.path)}${entry.description ? ` - ${entry.description}` : ""}`,
		entry.sourcePath ? `  - source: ${displayJournalPath(entry.sourcePath)}` : undefined,
		entry.routeLabel ? `  - route: ${entry.routeLabel}` : undefined,
		...uniquePaths(entry.artifacts || [entry.path])
			.filter((path) => path !== entry.path && path !== entry.sourcePath)
			.map((path) => `  - ${displayJournalPath(path)}`),
		"",
	].filter((line): line is string => line !== undefined);
}

function agentSnapshot(entry: AgentRecord): object {
	return {
		id: entry.id,
		label: entry.label,
		description: entry.description,
		path: entry.path ? displayJournalPath(entry.path) : undefined,
		beforePath: entry.beforePath ? displayJournalPath(entry.beforePath) : undefined,
		afterPath: entry.afterPath ? displayJournalPath(entry.afterPath) : undefined,
	};
}

function agentJournalLines(entry: AgentRecord): string[] {
	return [
		`- ${entry.label}${entry.description ? ` - ${entry.description}` : ""}`,
		entry.path ? `  - path: ${displayJournalPath(entry.path)}` : undefined,
		entry.beforePath ? `  - before: ${displayJournalPath(entry.beforePath)}` : undefined,
		entry.afterPath ? `  - after: ${displayJournalPath(entry.afterPath)}` : undefined,
	].filter((line): line is string => line !== undefined);
}

function activitySnapshot(entry: ActivityRecord): object {
	const paths = uniquePaths(entry.paths || (entry.path ? [entry.path] : []));
	return {
		id: entry.id,
		label: entry.label,
		description: entry.description,
		evidence: entry.evidence,
		path: entry.path ? displayJournalPath(entry.path) : undefined,
		paths: paths.map(displayJournalPath),
	};
}

function activityJournalLines(entry: ActivityRecord): string[] {
	const paths = uniquePaths(entry.paths || (entry.path ? [entry.path] : []));
	return [
		`- ${entry.label}${entry.description ? ` - ${entry.description}` : ""}`,
		entry.evidence ? `  - evidence: ${entry.evidence}` : undefined,
		...paths.map((path) => `  - ${displayJournalPath(path)}`),
	].filter((line): line is string => line !== undefined);
}

function displayJournalPath(path: string): string {
	return path.startsWith("/") || path.startsWith("#") ? path : `/${path}`;
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
		case "tour":
			return new vscode.ThemeIcon("checklist");
		case "checks":
			return new vscode.ThemeIcon("testing-view-icon");
		case "reports":
			return new vscode.ThemeIcon("notebook");
		case "dataStores":
			return new vscode.ThemeIcon("database");
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

function httpAppRouteId(name: string): string {
	return `http-app:${name}`;
}

function isIndexedHttpAppRoute(route: RouteRecord): boolean {
	return route.id.startsWith("http-app:");
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

function tourIcon(status: TourStatus): string {
	switch (status) {
		case "running":
			return "loading";
		case "ok":
			return "pass";
		case "failed":
			return "error";
		case "report":
			return "notebook";
	}
}

function checkIcon(status: CheckStatus): string {
	switch (status) {
		case "running":
			return "loading";
		case "ok":
			return "pass";
		case "warn":
			return "warning";
		case "failed":
			return "error";
		case "report":
			return "notebook";
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
