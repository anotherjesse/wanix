import * as vscode from 'vscode';

export async function fetchWorkbenchAsset(context: vscode.ExtensionContext, assetPath: string): Promise<Uint8Array> {
	const urls = assetUrls(context, assetPath);
	let lastError: unknown;
	for (const url of urls) {
		try {
			const response = await fetch(url, { cache: "no-store" });
			if (!response.ok) {
				lastError = new Error(`${url} returned HTTP ${response.status}`);
				continue;
			}
			return new Uint8Array(await response.arrayBuffer());
		} catch (error) {
			lastError = error;
		}
	}
	throw new Error(`Could not load ${assetPath}: ${lastError instanceof Error ? lastError.message : String(lastError)}`);
}

function assetUrls(context: vscode.ExtensionContext, assetPath: string): string[] {
	const base = context.extensionUri.toString();
	const slashBase = base.endsWith("/") ? base : `${base}/`;
	return [...new Set([
		new URL(assetPath, slashBase).toString(),
		new URL(`/workbench/${assetPath}`, base).toString(),
	])];
}
