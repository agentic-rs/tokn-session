import type { RelayEvent } from "@tokn/discord-pet/protocol";

import type { MatchConfig, RouteConfig } from "./config";

interface CompiledRoute {
  forward_to: string[];
  matches: (event: RelayEvent) => boolean;
}

export class RuleEngine {
  readonly #routes: CompiledRoute[];

  constructor(routes: RouteConfig[]) {
    this.#routes = routes.map((route) => ({
      forward_to: route.forward_to,
      matches: compileMatch(route.when)
    }));
  }

  targets(event: RelayEvent): string[] {
    const targets = new Set<string>();
    for (const route of this.#routes) {
      if (route.matches(event)) {
        for (const target of route.forward_to) {
          targets.add(target);
        }
      }
    }
    return [...targets];
  }
}

function compileMatch(match?: MatchConfig): (event: RelayEvent) => boolean {
  if (!match) {
    return () => true;
  }
  const repositoryPatterns = match.repository_names?.map(compileGlob);
  return (event) => {
    if (
      match.root_only
      && event.session.parent_session_id !== null
      && event.session.parent_session_id !== undefined
    ) {
      return false;
    }
    if (match.providers && !includes(match.providers, event.session.provider)) {
      return false;
    }
    if (match.event_types && !match.event_types.includes(event.event.type)) {
      return false;
    }
    if (match.roles && !includes(match.roles, stringField(event.event.role))) {
      return false;
    }
    if (
      match.deliveries
      && !includes(match.deliveries, stringField(event.event.delivery))
    ) {
      return false;
    }
    if (repositoryPatterns) {
      const repositoryName = event.session.project?.repository_name;
      if (
        !repositoryName
        || !repositoryPatterns.some((pattern) => pattern.test(repositoryName))
      ) {
        return false;
      }
    }
    return true;
  };
}

function compileGlob(glob: string): RegExp {
  let pattern = "^";
  for (const character of glob) {
    if (character === "*") {
      pattern += ".*";
    } else if (character === "?") {
      pattern += ".";
    } else {
      pattern += character.replace(/[\\^$.*+?()[\]{}|]/g, "\\$&");
    }
  }
  return new RegExp(`${pattern}$`, "i");
}

function includes(values: string[], value: string | undefined): boolean {
  return value !== undefined && values.includes(value);
}

function stringField(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}
