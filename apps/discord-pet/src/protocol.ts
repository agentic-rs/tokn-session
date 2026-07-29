export type JsonObject = Record<string, unknown>;

export interface RelaySession {
  provider?: string;
  session_id: string;
  parent_session_id?: string | null;
  title?: string | null;
  project?: {
    name?: string | null;
    project_name?: string | null;
    folder_name?: string | null;
    repository_name?: string | null;
  } | null;
}

export interface AgentEvent extends JsonObject {
  type: string;
}

export interface RelayEvent {
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

  const parsed: RelayEvent = {
    topic,
    session: {
      session_id: sessionId
    },
    event: {
      ...event,
      type: eventType
    }
  };
  const provider = asString(session.provider);
  if (provider) {
    parsed.session.provider = provider;
  }
  const parentSessionId = asNullableString(session.parent_session_id);
  if (parentSessionId !== undefined) {
    parsed.session.parent_session_id = parentSessionId;
  }
  const title = asNullableString(session.title);
  if (title !== undefined) {
    parsed.session.title = title;
  }
  const project = parseProject(session.project);
  if (project !== undefined) {
    parsed.session.project = project;
  }
  return parsed;
}

export function asObject(value: unknown): JsonObject | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonObject
    : null;
}

export function asString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function asNullableString(value: unknown): string | null | undefined {
  if (value === null) {
    return null;
  }
  return asString(value);
}

function parseProject(value: unknown): RelaySession["project"] | undefined {
  if (value === null) {
    return null;
  }
  const project = asObject(value);
  if (!project) {
    return undefined;
  }
  const parsed: NonNullable<RelaySession["project"]> = {};
  assignNullableString(parsed, "name", project.name);
  assignNullableString(parsed, "project_name", project.project_name);
  assignNullableString(parsed, "folder_name", project.folder_name);
  assignNullableString(parsed, "repository_name", project.repository_name);
  return parsed;
}

function assignNullableString(
  target: NonNullable<RelaySession["project"]>,
  field: keyof NonNullable<RelaySession["project"]>,
  value: unknown
): void {
  const parsed = asNullableString(value);
  if (parsed !== undefined) {
    target[field] = parsed;
  }
}
