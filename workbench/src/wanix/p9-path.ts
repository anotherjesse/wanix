export function splitPath(name: string): string[] {
	const normalized = normalizePath(name);
	return normalized === "/" ? [] : normalized.split("/").filter(Boolean);
}

export function normalizePath(name: string): string {
	if (!name || name === ".") {
		return "/";
	}
	return name.startsWith("/") ? name : `/${name}`;
}

export function parentPath(name: string): string {
	const parts = splitPath(name);
	parts.pop();
	return parts.length === 0 ? "/" : `/${parts.join("/")}`;
}

export function baseName(name: string): string {
	const parts = splitPath(name);
	const base = parts.pop();
	if (!base) {
		throw new Error("path must name an entry");
	}
	return base;
}

export function baseNameOrRoot(name: string): string {
	if (splitPath(name).length === 0) {
		return "/";
	}
	return baseName(name);
}

export function joinPath(parent: string, child: string): string {
	const normalized = normalizePath(parent);
	return normalized === "/" ? `/${child}` : `${normalized}/${child}`;
}
