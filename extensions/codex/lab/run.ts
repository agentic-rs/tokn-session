import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { CodexDesktopInputClient } from "../lib/desktop-input-client";
import type { CodexDesktopStartTurnRequest } from "../lib/ipc-protocol";
import { IsolatedCodexAppServer, type AppServerTurnHandle } from "./app-server";
import { FakeCodexDesktopOwner, FakeCodexDesktopRouter } from "./fake-desktop";

const MODEL = "deepseek-v4-flash";
const BASE_URL = "http://localhost:4141/v1";
const PROMPT = "Reply with exactly: codex pet bridge works";

const directory = await mkdtemp(join(tmpdir(), "tokn-codex-lab-"));
const ipcDirectory = join(directory, "ipc");
const socketPath = join(ipcDirectory, "ipc.sock");
const codexHome = join(directory, "codex-home");
const repository = join(directory, "repo");
await mkdir(ipcDirectory, { mode: 0o700 });

let router: FakeCodexDesktopRouter | undefined;
let owner: FakeCodexDesktopOwner | undefined;
let input: CodexDesktopInputClient | undefined;
let appServer: IsolatedCodexAppServer | undefined;

try {
  appServer = await IsolatedCodexAppServer.start({
    codex_home: codexHome,
    cwd: repository,
    model: MODEL,
    base_url: BASE_URL
  });
  router = new FakeCodexDesktopRouter(socketPath);
  await router.start();

  let turn: AppServerTurnHandle | undefined;
  owner = await FakeCodexDesktopOwner.connect({
    socket_path: socketPath,
    conversation_id: appServer.thread_id,
    start_turn: async (request) => {
      const prompt = textFromRequest(request);
      turn = await appServer?.startTurn(prompt);
      if (!turn) {
        throw new Error("isolated app-server did not start the turn");
      }
      return turn.response;
    }
  });
  input = await CodexDesktopInputClient.connect({
    socket_path: socketPath,
    timeout_ms: 10_000
  });
  const admission = await input.startTurn(appServer.thread_id, PROMPT);
  if (!turn) {
    throw new Error("fake desktop owner did not receive the start-turn request");
  }
  const completion = await (turn as AppServerTurnHandle).completion;

  console.log(JSON.stringify({
    model: MODEL,
    base_url: BASE_URL,
    socket_path: socketPath,
    codex_home: codexHome,
    conversation_id: appServer.thread_id,
    admission,
    completion
  }, null, 2));
} finally {
  input?.close();
  owner?.close();
  await router?.stop();
  await appServer?.close();
  await rm(directory, { recursive: true, force: true });
}

function textFromRequest(request: CodexDesktopStartTurnRequest): string {
  const item = request.params.turnStartParams.input.find((input) => input.type === "text");
  if (!item?.text) {
    throw new Error("desktop start-turn request omitted text input");
  }
  return item.text;
}
