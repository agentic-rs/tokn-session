/** Consume login credentials before React renders (including StrictMode remounts). */
export function consumeLoginToken(): string | undefined {
  return consumeFragmentCredential("token");
}

export function consumeBootstrapToken(): string | undefined {
  return consumeFragmentCredential("bootstrap_token");
}

function consumeFragmentCredential(key: string): string | undefined {
  const fragment = new URLSearchParams(window.location.hash.slice(1));
  if (!fragment.has(key)) return undefined;
  const token = fragment.get(key) || undefined;
  fragment.delete(key);
  const remaining = fragment.toString();
  window.history.replaceState(window.history.state, "", `${window.location.pathname}${window.location.search}${remaining ? `#${remaining}` : ""}`);
  return token;
}
