import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';
import { WanixSystemView } from './system-view.js';

export const AGENT_TOOL_CONTRACT_MD_PATH = ".wanix/agent-tools.md";
export const AGENT_TOOL_CONTRACT_JSON_PATH = ".wanix/agent-tools.json";

type AgentTool = {
	name: string;
	description: string;
	input: Record<string, string>;
	output: Record<string, string>;
	wanixSurface: string[];
};

const AGENT_TOOLS: AgentTool[] = [
	{
		name: "readFile",
		description: "Read a Wanix file as text before deciding what to do.",
		input: { path: "absolute or workspace-relative Wanix path" },
		output: { text: "file contents" },
		wanixSurface: ["wanix:/<path>", "#task service files when inspecting tasks"],
	},
	{
		name: "writeFile",
		description: "Write a Wanix file and let the workbench refresh/activity stream observe it.",
		input: { path: "Wanix path", text: "replacement file contents" },
		output: { path: "written Wanix path" },
		wanixSurface: ["wanix:/<path>", "Activity row with the touched path"],
	},
	{
		name: "runTask",
		description: "Run a qjs or wasm program through Wanix task and terminal services.",
		input: { kind: "qjs or wasm", path: "program path" },
		output: { taskId: "#task id", transcriptPath: "saved task transcript", metadataPath: "saved task metadata" },
		wanixSurface: ["#task/new", "#term/<id>/program", ".wanix/tasks/<id>-*.output.txt"],
	},
	{
		name: "observeTask",
		description: "Observe task exit, stdout/stderr transcript, metadata, and service files.",
		input: { taskId: "Wanix task id" },
		output: { exitCode: "numeric exit when available", transcript: "captured output text" },
		wanixSurface: ["#task/<id>/exit", "#task/<id>/kind", ".wanix/tasks/<id>-*.metadata.json"],
	},
	{
		name: "readShellHistory",
		description: "Read or summarize served qjs-shell command outcomes without subscribing to terminal bytes.",
		input: {},
		output: { commands: "append-only command history, latest shell outcome batch, and grouped summary artifact" },
		wanixSurface: [".wanix/qjs-shell/commands.jsonl", ".wanix/qjs-shell/latest.json", ".wanix/qjs-shell/latest.md", ".wanix/qjs-shell/summary.md", ".wanix/qjs-shell/selected.md"],
	},
	{
		name: "diffFiles",
		description: "Show a before/after source edit in the workbench diff view.",
		input: { beforePath: "snapshot before edit", afterPath: "snapshot after edit" },
		output: { opened: "browser-native diff view" },
		wanixSurface: ["wanix:/<beforePath>", "wanix:/<afterPath>"],
	},
	{
		name: "writeRepairReport",
		description: "Persist a structured agent repair run record for humans and future agent backends.",
		input: { target: "program path", operations: "ordered Wanix tool operations", status: "repaired, already repaired, or failed" },
		output: { markdownPath: "human report", jsonPath: "wanix.agent-repair.v1 run record" },
		wanixSurface: ["/agent/out/*.repair-report.md", "/agent/out/*.repair-report.json", "Reports section artifacts"],
	},
	{
		name: "previewHttpRoute",
		description: "Run a Wanix-backed HTTP route and save response plus route-run artifacts.",
		input: { route: "/.wanix/app/<name>" },
		output: { responsePath: "saved response report", artifacts: "route-run trace files" },
		wanixSurface: ["/.wanix/app/<name>", ".wanix/http/<task>.out", ".wanix/http/<task>.err"],
	},
	{
		name: "writeSystemSnapshot",
		description: "Persist the current cockpit state for humans and agents.",
		input: {},
		output: { journalPath: "/.wanix/system-journal.md", statePath: "/.wanix/system-state.json" },
		wanixSurface: [".wanix/system-journal.md", ".wanix/system-state.json"],
	},
	{
		name: "openPath",
		description: "Open a Wanix file or inspect a Wanix service directory in the browser workbench.",
		input: { path: "Wanix path or service path" },
		output: { opened: "editor tab or service inspector" },
		wanixSurface: ["wanix:/<path>", "#task", "#term"],
	},
];

export async function openAgentToolContract(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<void> {
	await writeAgentToolContract(fsys, bridge, systemView);
	await openWanixFile(AGENT_TOOL_CONTRACT_MD_PATH);
	vscode.window.showInformationMessage("Opened Wanix agent tool contract");
}

export async function writeAgentToolContract(
	fsys: any,
	bridge: WanixBridge,
	systemView: WanixSystemView,
): Promise<{ markdownPath: string; jsonPath: string }> {
	const generatedAt = new Date();
	systemView.agentStarted("publish agent tool contract");
	systemView.agentStep("write agent-tools.json", { icon: "json", path: AGENT_TOOL_CONTRACT_JSON_PATH });
	systemView.agentStep("write agent-tools.md", { icon: "notebook", path: AGENT_TOOL_CONTRACT_MD_PATH });
	await fsys.makeDirAll(".wanix");
	await fsys.writeFile(AGENT_TOOL_CONTRACT_JSON_PATH, agentToolContractJson(generatedAt));
	await fsys.writeFile(AGENT_TOOL_CONTRACT_MD_PATH, agentToolContractMarkdown(generatedAt));
	bridge.refresh("/.wanix");
	bridge.refresh(`/${AGENT_TOOL_CONTRACT_JSON_PATH}`);
	bridge.refresh(`/${AGENT_TOOL_CONTRACT_MD_PATH}`);
	systemView.filesystemActivity("agent tool contract written", {
		path: AGENT_TOOL_CONTRACT_MD_PATH,
		paths: [AGENT_TOOL_CONTRACT_JSON_PATH, AGENT_TOOL_CONTRACT_MD_PATH],
	});
	systemView.reportPublished("Agent Tool Contract", AGENT_TOOL_CONTRACT_MD_PATH, {
		kind: "agent",
		description: "agent tools",
		icon: "symbol-method",
		artifacts: [AGENT_TOOL_CONTRACT_MD_PATH, AGENT_TOOL_CONTRACT_JSON_PATH],
	});
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	return {
		markdownPath: AGENT_TOOL_CONTRACT_MD_PATH,
		jsonPath: AGENT_TOOL_CONTRACT_JSON_PATH,
	};
}

function agentToolContractJson(generatedAt: Date): string {
	return `${JSON.stringify({
		schema: "wanix.agent-tools.v1",
		generatedAt: generatedAt.toISOString(),
		markdownPath: `/${AGENT_TOOL_CONTRACT_MD_PATH}`,
		jsonPath: `/${AGENT_TOOL_CONTRACT_JSON_PATH}`,
		systemStatePath: "/.wanix/system-state.json",
		systemJournalPath: "/.wanix/system-journal.md",
		backend: {
			current: "deterministic local repair",
			next: "Codex app-server can attach behind this Wanix-shaped tool contract",
		},
		serviceRoots: ["#task", "#term"],
		tools: AGENT_TOOLS,
		repairDemo: {
			target: "/agent/broken.js",
			reportPath: "/agent/out/broken.repair-report.md",
			reportJsonPath: "/agent/out/broken.repair-report.json",
			reportSchema: "wanix.agent-repair.v1",
			expectedResultPath: "/agent/out/result.txt",
			loop: ["readFile", "runTask", "observeTask", "writeFile", "diffFiles", "runTask", "observeTask", "writeRepairReport", "openPath"],
		},
		policies: [
			"Operate through Wanix files, tasks, terminals, service files, and route artifacts.",
			"Keep task cancellation and control semantics inside #term/#task contracts.",
			"Treat served HTTP app routes as loopback/local until auth is explicitly designed.",
			"Write durable reports for repairs and snapshots so work remains inspectable.",
		],
	}, null, 2)}\n`;
}

function agentToolContractMarkdown(generatedAt: Date): string {
	return [
		"# Wanix Agent Tool Contract",
		"",
		`Generated: ${generatedAt.toISOString()}`,
		`Schema: wanix.agent-tools.v1`,
		`JSON: /${AGENT_TOOL_CONTRACT_JSON_PATH}`,
		`System state: /.wanix/system-state.json`,
		"",
		"## Purpose",
		"",
		"This is the Wanix-shaped tool surface an agent should use from the browser cockpit. The current repair demo uses a deterministic local backend, but the same operations are the boundary a Codex app-server engine can attach to later.",
		"",
		"## Tools",
		"",
		...AGENT_TOOLS.flatMap(agentToolMarkdown),
		"## Repair Demo Loop",
		"",
		"1. readFile /agent/broken.js",
		"2. runTask qjs /agent/broken.js",
		"3. observeTask transcript and exit",
		"4. writeFile repaired source plus before/after snapshots",
		"5. runTask qjs /agent/broken.js again",
		"6. observeTask success and result file",
		"7. writeRepairReport /agent/out/broken.repair-report.md and /agent/out/broken.repair-report.json",
		"8. openPath /agent/out/broken.repair-report.md",
		"",
		"The JSON report uses schema `wanix.agent-repair.v1` and records the ordered Wanix operations, task ids, transcript paths, metadata paths, before/after snapshots, result path, and status.",
		"",
		"## Policies",
		"",
		"- Operate through Wanix files, tasks, terminals, service files, and route artifacts.",
		"- Keep task cancellation and control semantics inside #term/#task contracts.",
		"- Treat served HTTP app routes as loopback/local until auth is explicitly designed.",
		"- Write durable reports for repairs and snapshots so work remains inspectable.",
		"",
	].join("\n");
}

function agentToolMarkdown(tool: AgentTool): string[] {
	return [
		`### ${tool.name}`,
		"",
		tool.description,
		"",
		`- input: ${formatShape(tool.input)}`,
		`- output: ${formatShape(tool.output)}`,
		`- Wanix surface: ${tool.wanixSurface.join(", ")}`,
		"",
	];
}

function formatShape(shape: Record<string, string>): string {
	const entries = Object.entries(shape);
	if (entries.length === 0) {
		return "none";
	}
	return entries.map(([key, value]) => `${key} (${value})`).join("; ");
}

async function openWanixFile(path: string): Promise<void> {
	const document = await vscode.workspace.openTextDocument(vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path: path.startsWith("/") ? path : `/${path}`,
	}));
	await vscode.window.showTextDocument(document, { preview: false });
}
