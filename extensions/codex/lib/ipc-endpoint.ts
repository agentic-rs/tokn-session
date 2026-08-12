import { join, resolve } from "node:path";

export const CODEX_WINDOWS_PIPE_NAME = String.raw`\\.\pipe\codex-ipc`;

export type CodexIpcEndpoint =
  | {
      transport: "unix_socket";
      path: string;
    }
  | {
      transport: "windows_pipe";
      pipe_name: string;
    };

export function codexDesktopIpcEndpoint(
  codexHome: string,
  platform: NodeJS.Platform = process.platform
): CodexIpcEndpoint {
  if (platform === "win32") {
    return {
      transport: "windows_pipe",
      pipe_name: CODEX_WINDOWS_PIPE_NAME
    };
  }
  return {
    transport: "unix_socket",
    path: join(resolve(codexHome), "ipc", "ipc.sock")
  };
}

export function ipcEndpointAddress(endpoint: CodexIpcEndpoint): string {
  switch (endpoint.transport) {
    case "unix_socket": {
      const path = endpoint.path.trim();
      if (!path) {
        throw new Error("Codex Unix socket path is required");
      }
      return path;
    }
    case "windows_pipe": {
      const pipeName = endpoint.pipe_name.trim();
      if (!pipeName) {
        throw new Error("Codex Windows pipe name is required");
      }
      if (!pipeName.toLowerCase().startsWith("\\\\.\\pipe\\")) {
        throw new Error("Codex Windows pipe name must use the local named-pipe namespace");
      }
      return pipeName;
    }
  }
}
