export type JsonObject = Record<string, unknown>;

export interface RelaySession {
  provider?: string;
  session_id: string;
  parent_session_id?: string | null;
  agent_path?: string | null;
  agent_nickname?: string | null;
  agent_role?: string | null;
  title?: string | null;
  cwd?: string | null;
  project?: {
    name?: string | null;
    project_name?: string | null;
    folder?: string | null;
    folder_name?: string | null;
    repository_name?: string | null;
  } | null;
}

export interface AgentEvent extends JsonObject {
  type: string;
  provider?: string;
  session_id?: string | null;
  timestamp?: string | null;
}

export interface RelayEvent {
  path?: string;
  topic: string;
  session: RelaySession;
  event: AgentEvent;
}

export function parseRelayEvent(value: unknown): RelayEvent | null {
  const record = asObject(value);
  const session = asObject(record?.session);
  const event = asObject(record?.event);
  const topic = asString(record?.topic);
  const sessionId = asString(session?.session_id);
  const eventType = asString(event?.type);

  if (!record || !session || !event || !topic || !sessionId || !eventType) {
    return null;
  }

  return {
    path: asString(record.path),
    topic,
    session: {
      provider: asString(session.provider),
      session_id: sessionId,
      parent_session_id: asNullableString(session.parent_session_id),
      agent_path: asNullableString(session.agent_path),
      agent_nickname: asNullableString(session.agent_nickname),
      agent_role: asNullableString(session.agent_role),
      title: asNullableString(session.title),
      cwd: asNullableString(session.cwd),
      project: parseProject(session.project)
    },
    event: {
      ...event,
      type: eventType
    }
  };
}

export function asObject(value: unknown): JsonObject | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonObject
    : null;
}

export function asString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

export function asNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function asNullableString(value: unknown): string | null | undefined {
  if (value === null) {
    return null;
  }
  return asString(value);
}

function parseProject(value: unknown): RelaySession["project"] {
  if (value === null) {
    return null;
  }
  const project = asObject(value);
  if (!project) {
    return undefined;
  }
  return {
    name: asNullableString(project.name),
    project_name: asNullableString(project.project_name),
    folder: asNullableString(project.folder),
    folder_name: asNullableString(project.folder_name),
    repository_name: asNullableString(project.repository_name)
  };
}
