// Copyright (c) {{license_year}} {{author_name}}.
//
// {{mcp_crate_name}} VS Code extension entry point.
//
// Registers the bundled `{{mcp_crate_name}}` binary as an MCP server so that
// Copilot Chat (and any other VS Code MCP consumer) discovers it automatically
// with no `.vscode/mcp.json` editing required.

import * as vscode from "vscode";
import * as fs from "fs";
import * as path from "path";

const PROVIDER_ID = "{{mcp_crate_name}}";
const SERVER_LABEL = "{{mcp_crate_name}}";
const CONFIG_SECTION = "{{mcp_crate_name}}";
const BINARY_BASENAME = "{{mcp_crate_name}}";

function resolveBinaryPath(context: vscode.ExtensionContext): string | undefined {
    const config = vscode.workspace.getConfiguration(CONFIG_SECTION);

    const override = (config.get<string>("binaryPath") ?? "").trim();
    if (override.length > 0) {
        if (fs.existsSync(override)) {
            return override;
        }
        console.warn(
            `[${CONFIG_SECTION}] ${CONFIG_SECTION}.binaryPath = ${override} does not exist; ` +
                "falling back to bundled binary.",
        );
    }

    const binaryName =
        process.platform === "win32" ? `${BINARY_BASENAME}.exe` : BINARY_BASENAME;
    const bundled = path.join(context.extensionPath, "bin", binaryName);
    if (fs.existsSync(bundled)) {
        return bundled;
    }
    return undefined;
}

function buildArgs(): string[] {
    const config = vscode.workspace.getConfiguration(CONFIG_SECTION);
    const extraArgs = config.get<string[]>("extraArgs", []) ?? [];
    return extraArgs.filter((a) => typeof a === "string" && a.length > 0);
}

function readBinaryVersion(context: vscode.ExtensionContext, binary: string): string {
    const bundledDir = path.join(context.extensionPath, "bin");
    const isBundled =
        path.normalize(path.dirname(binary)).toLowerCase() ===
        path.normalize(bundledDir).toLowerCase();

    if (isBundled) {
        const v = readVersionFile(path.join(bundledDir, "VERSION"));
        if (v !== undefined) return v;
        return context.extension.packageJSON.version ?? "0.0.0";
    }

    const sibling = readVersionFile(path.join(path.dirname(binary), "VERSION"));
    if (sibling !== undefined) return `${sibling} (override)`;
    return "override";
}

function readVersionFile(versionFile: string): string | undefined {
    try {
        if (fs.existsSync(versionFile)) {
            const v = fs.readFileSync(versionFile, "utf8").trim();
            if (v.length > 0) return v;
        }
    } catch {
        // fall through
    }
    return undefined;
}

class McpServerProvider
    implements vscode.McpServerDefinitionProvider<vscode.McpStdioServerDefinition>
{
    private readonly _onDidChange = new vscode.EventEmitter<void>();
    public readonly onDidChangeMcpServerDefinitions = this._onDidChange.event;

    private missingBinaryWarned = false;

    constructor(private readonly context: vscode.ExtensionContext) {
        const sub = vscode.workspace.onDidChangeConfiguration((e) => {
            if (e.affectsConfiguration(CONFIG_SECTION)) {
                this.missingBinaryWarned = false;
                this._onDidChange.fire();
            }
        });
        context.subscriptions.push(sub, this._onDidChange);
    }

    public provideMcpServerDefinitions(
        _token: vscode.CancellationToken,
    ): vscode.ProviderResult<vscode.McpStdioServerDefinition[]> {
        const binary = resolveBinaryPath(this.context);
        if (binary === undefined) {
            if (!this.missingBinaryWarned) {
                this.missingBinaryWarned = true;
                void vscode.window.showWarningMessage(
                    `${CONFIG_SECTION}: bundled server binary not found. ` +
                        `Reinstall the extension or set '${CONFIG_SECTION}.binaryPath'.`,
                );
            }
            return [];
        }
        const version = readBinaryVersion(this.context, binary);
        return [
            new vscode.McpStdioServerDefinition(
                SERVER_LABEL,
                binary,
                buildArgs(),
                /* env */ {},
                version,
            ),
        ];
    }

    public resolveMcpServerDefinition(
        server: vscode.McpStdioServerDefinition,
        _token: vscode.CancellationToken,
    ): vscode.ProviderResult<vscode.McpStdioServerDefinition> {
        return server;
    }
}

export function activate(context: vscode.ExtensionContext): void {
    const provider = new McpServerProvider(context);
    context.subscriptions.push(
        vscode.lm.registerMcpServerDefinitionProvider(PROVIDER_ID, provider),
    );

    context.subscriptions.push(
        vscode.commands.registerCommand(`${CONFIG_SECTION}.copyServerPath`, async () => {
            const binary = resolveBinaryPath(context);
            if (binary === undefined) {
                await vscode.window.showErrorMessage(
                    `${CONFIG_SECTION}: bundled server binary not found.`,
                );
                return;
            }
            await vscode.env.clipboard.writeText(binary);
            await vscode.window.showInformationMessage(
                `${CONFIG_SECTION}: copied server path to clipboard: ${binary}`,
            );
        }),
    );

    context.subscriptions.push(
        vscode.commands.registerCommand(`${CONFIG_SECTION}.showServerVersion`, async () => {
            const binary = resolveBinaryPath(context);
            if (binary === undefined) {
                await vscode.window.showInformationMessage(
                    `${CONFIG_SECTION} server: binary not found`,
                );
                return;
            }
            const version = readBinaryVersion(context, binary);
            await vscode.window.showInformationMessage(
                `${CONFIG_SECTION} server version ${version} \u2014 ${binary}`,
            );
        }),
    );
}

export function deactivate(): void {
    // All disposables are managed via context.subscriptions.
}
