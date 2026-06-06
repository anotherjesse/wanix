import * as vscode from 'vscode';
import { WanixBridge } from './bridge.js';
import { WanixSystemView } from './system-view.js';

const SHARED_DIR = "/shared";
const README_PATH = `${SHARED_DIR}/README.md`;
const MESSAGE_PATH = `${SHARED_DIR}/message.txt`;
const LINUX_PATH = `${SHARED_DIR}/from-linux.txt`;

const STARTER_MESSAGE = "hello from Wanix workbench\n";
const LINUX_PLACEHOLDER = "Linux/v86 has not written here yet.\n";

export type V86SharedConfig = {
	launchUrl?: string;
	boot?: {
		ready?: boolean;
		kernel?: string;
		init?: string;
		initrd?: string;
		missing?: string[];
	};
	rootfs?: {
		status?: string;
		ready?: boolean;
		url?: string;
		missing?: string[];
	};
	p9?: {
		websocket?: string;
	};
	defaultCmdline?: string;
	p9Msize?: number;
};

export type V86SharedDemoConfig = {
	v86?: V86SharedConfig;
};

export async function openV86SharedDemo(
	fsys: any,
	bridge: WanixBridge,
	config: V86SharedDemoConfig,
	systemView: WanixSystemView,
	options: { openReadme?: boolean; notify?: boolean } = {},
): Promise<void> {
	const openReadme = options.openReadme ?? true;
	const notify = options.notify ?? true;
	await fsys.makeDirAll(SHARED_DIR);
	await fsys.writeFile(README_PATH, readme(config.v86));
	const installedMessage = await writeTextIfMissing(fsys, MESSAGE_PATH, STARTER_MESSAGE);
	const installedLinuxPlaceholder = await writeTextIfMissing(fsys, LINUX_PATH, LINUX_PLACEHOLDER);
	bridge.refresh();
	systemView.filesystemActivity(installedMessage || installedLinuxPlaceholder
		? "v86 shared files demo installed"
		: "v86 shared files demo opened");
	await refreshWorkbenchViews();
	if (openReadme) {
		await openWanixFile(README_PATH);
	}
	if (notify) {
		vscode.window.showInformationMessage("Opened Wanix v86 shared files demo in /shared");
	}
}

export async function openDirectV86(
	config: V86SharedDemoConfig,
	systemView: WanixSystemView,
): Promise<void> {
	const url = config.v86?.launchUrl;
	if (!url) {
		throw new Error("Wanix discovery did not advertise a direct-v86 launch URL");
	}
	await vscode.env.openExternal(vscode.Uri.parse(url));
	systemView.filesystemActivity("direct-v86 launch opened");
}

function readme(v86: V86SharedConfig | undefined): string {
	const launchUrl = v86?.launchUrl || "not advertised";
	const p9 = v86?.p9?.websocket || "not advertised";
	const msize = v86?.p9Msize || 131072;
	return `# Wanix v86 Shared Files

This starter sets up the files used by the Linux/v86 shared-file demo.

Wanix files:

\`\`\`text
${MESSAGE_PATH}
${LINUX_PATH}
\`\`\`

direct-v86 launch:

\`\`\`text
${launchUrl}
\`\`\`

Boot status:

\`\`\`text
${bootStatus(v86)}
\`\`\`

9P route:

\`\`\`text
${p9}
\`\`\`

Workbench side:

1. Edit ${MESSAGE_PATH}.
2. Launch direct-v86.
3. After Linux boots, read the same file:

\`\`\`sh
cat /shared/message.txt
printf 'from linux\\n' > /shared/from-linux.txt
\`\`\`

If the guest is mounting Wanix separately instead of using it as root:

\`\`\`sh
mkdir -p /mnt/wanix
mount -t 9p -o trans=virtio,version=9p2000.L,msize=${msize},cache=none host9p /mnt/wanix
cat /mnt/wanix/shared/message.txt
printf 'from linux\\n' > /mnt/wanix/shared/from-linux.txt
\`\`\`

Back in the workbench, refresh Explorer and open ${LINUX_PATH}.
`;
}

function bootStatus(v86: V86SharedConfig | undefined): string {
	const boot = v86?.boot || {};
	const rootfs = v86?.rootfs || {};
	const bootLine = boot.ready
		? `boot ready: kernel ${boot.kernel || "unknown"}, init ${boot.init || "unknown"}`
		: `boot not ready: ${missingList(boot.missing)}`;
	const rootfsLine = `rootfs route: ${rootfs.status || "unknown"}${rootfs.ready ? " ready" : ""}`;
	return `${bootLine}\n${rootfsLine}`;
}

function missingList(missing: string[] | undefined): string {
	return missing?.length ? missing.join(", ") : "missing boot markers";
}

async function writeTextIfMissing(fsys: any, path: string, contents: string): Promise<boolean> {
	if (await wanixPathExists(fsys, path)) {
		return false;
	}
	await fsys.writeFile(path, contents);
	return true;
}

async function wanixPathExists(fsys: any, path: string): Promise<boolean> {
	try {
		await fsys.stat(path);
		return true;
	} catch {
		return false;
	}
}

async function refreshWorkbenchViews(): Promise<void> {
	await Promise.resolve(vscode.commands.executeCommand("workbench.files.action.refreshFilesExplorer")).catch((error: unknown) => {
		console.warn("Wanix explorer refresh failed", error);
	});
	await Promise.resolve(vscode.commands.executeCommand("workbench.view.extension.wanix")).catch((error: unknown) => {
		console.warn("Wanix system view focus failed", error);
	});
}

async function openWanixFile(path: string): Promise<void> {
	const document = await vscode.workspace.openTextDocument(vscode.Uri.from({
		scheme: WanixBridge.scheme,
		path,
	}));
	await vscode.window.showTextDocument(document, { preview: false });
}
